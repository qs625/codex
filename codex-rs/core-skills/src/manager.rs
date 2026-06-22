use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;

use codex_file_system::ExecutorFileSystem;
use codex_protocol::protocol::Product;
use codex_protocol::protocol::SkillScope;
use codex_utils_absolute_path::AbsolutePathBuf;
use tracing::info;

use crate::config_rules::resolve_disabled_skill_paths;
use crate::config_rules::skill_config_rules_from_stack;
use crate::loader::SkillRoot;
use crate::loader::load_skills_from_roots;
use crate::loader::skill_roots;
use crate::system::install_system_skills;
use crate::system::uninstall_system_skills;
use codex_core_skills_api::SkillLoadOutcome;
use codex_core_skills_api::SkillsLoadInput;
use codex_core_skills_api::SkillsRuntime;
use codex_core_skills_api::SkillsRuntimeFuture;
use codex_core_skills_api::build_implicit_skill_path_indexes;
use codex_core_skills_api::bundled_skills_enabled_from_stack;
use codex_core_skills_api::config_rules::SkillConfigRules;

pub struct SkillsManager {
    codex_home: AbsolutePathBuf,
    restriction_product: Option<Product>,
    cache_by_cwd: RwLock<HashMap<AbsolutePathBuf, SkillLoadOutcome>>,
    cache_by_config: RwLock<HashMap<ConfigSkillsCacheKey, SkillLoadOutcome>>,
}

impl SkillsManager {
    pub fn new(codex_home: AbsolutePathBuf, bundled_skills_enabled: bool) -> Self {
        Self::new_with_restriction_product(codex_home, bundled_skills_enabled, Some(Product::Codex))
    }

    pub fn new_with_restriction_product(
        codex_home: AbsolutePathBuf,
        bundled_skills_enabled: bool,
        restriction_product: Option<Product>,
    ) -> Self {
        let manager = Self {
            codex_home,
            restriction_product,
            cache_by_cwd: RwLock::new(HashMap::new()),
            cache_by_config: RwLock::new(HashMap::new()),
        };
        if !bundled_skills_enabled {
            // The loader caches bundled skills under `skills/.system`. Clearing that directory is
            // best-effort cleanup; root selection still enforces the config even if removal fails.
            uninstall_system_skills(&manager.codex_home);
        } else if let Err(err) = install_system_skills(&manager.codex_home) {
            tracing::error!("failed to install system skills: {err}");
        }
        manager
    }

    /// Load skills for an already-constructed [`Config`], avoiding any additional config-layer
    /// loading.
    ///
    /// This path uses a cache keyed by the effective skill-relevant config state rather than just
    /// cwd so role-local and session-local skill overrides cannot bleed across sessions that happen
    /// to share a directory.
    pub async fn skills_for_config(
        &self,
        input: &SkillsLoadInput,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillLoadOutcome {
        let roots = self.skill_roots_for_config(input, fs).await;
        let skill_config_rules = skill_config_rules_from_stack(&input.config_layer_stack);
        let cache_key =
            config_skills_cache_key(&roots, &skill_config_rules, &input.allowlist_patterns);
        if let Some(outcome) = self.cached_outcome_for_config(&cache_key) {
            return outcome;
        }

        let outcome = self
            .build_skill_outcome(
                roots,
                &skill_config_rules,
                input.allowlist_patterns.as_deref(),
            )
            .await;
        let mut cache = self
            .cache_by_config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.insert(cache_key, outcome.clone());
        outcome
    }

    pub async fn skill_roots_for_config(
        &self,
        input: &SkillsLoadInput,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> Vec<SkillRoot> {
        let mut roots = skill_roots(
            fs,
            &input.config_layer_stack,
            &input.cwd,
            input.effective_skill_roots.clone(),
        )
        .await;
        if !input.bundled_skills_enabled {
            roots.retain(|root| root.scope != SkillScope::System);
        }
        roots
    }

    pub async fn skills_for_cwd(
        &self,
        input: &SkillsLoadInput,
        force_reload: bool,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillLoadOutcome {
        let use_cwd_cache = fs.is_some();
        if use_cwd_cache
            && !force_reload
            && let Some(outcome) = self.cached_outcome_for_cwd(&input.cwd)
        {
            return outcome;
        }

        let mut roots = skill_roots(
            fs.clone(),
            &input.config_layer_stack,
            &input.cwd,
            input.effective_skill_roots.clone(),
        )
        .await;
        if !bundled_skills_enabled_from_stack(&input.config_layer_stack) {
            roots.retain(|root| root.scope != SkillScope::System);
        }
        let skill_config_rules = skill_config_rules_from_stack(&input.config_layer_stack);
        let outcome = self
            .build_skill_outcome(
                roots,
                &skill_config_rules,
                input.allowlist_patterns.as_deref(),
            )
            .await;
        if use_cwd_cache {
            let mut cache = self
                .cache_by_cwd
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.insert(input.cwd.clone(), outcome.clone());
        }
        outcome
    }

    async fn build_skill_outcome(
        &self,
        roots: Vec<SkillRoot>,
        skill_config_rules: &SkillConfigRules,
        allowlist_patterns: Option<&[String]>,
    ) -> SkillLoadOutcome {
        let outcome = crate::filter_skill_load_outcome_for_product(
            load_skills_from_roots(roots).await,
            self.restriction_product,
        );
        let mut disabled_paths = resolve_disabled_skill_paths(&outcome.skills, skill_config_rules);
        if let Some(allowlist_patterns) = allowlist_patterns {
            disabled_paths.extend(
                outcome
                    .skills
                    .iter()
                    .filter(|skill| {
                        !allowlist_patterns
                            .iter()
                            .any(|pattern| skill_matches_pattern(skill, pattern))
                    })
                    .map(|skill| skill.path_to_skills_md.clone()),
            );
        }
        finalize_skill_outcome(outcome, disabled_paths)
    }

    pub fn clear_cache(&self) {
        let cleared_cwd = {
            let mut cache = self
                .cache_by_cwd
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let cleared = cache.len();
            cache.clear();
            cleared
        };
        let cleared_config = {
            let mut cache = self
                .cache_by_config
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let cleared = cache.len();
            cache.clear();
            cleared
        };
        let cleared = cleared_cwd + cleared_config;
        info!("skills cache cleared ({cleared} entries)");
    }

    fn cached_outcome_for_cwd(&self, cwd: &AbsolutePathBuf) -> Option<SkillLoadOutcome> {
        match self.cache_by_cwd.read() {
            Ok(cache) => cache.get(cwd).cloned(),
            Err(err) => err.into_inner().get(cwd).cloned(),
        }
    }

    fn cached_outcome_for_config(
        &self,
        cache_key: &ConfigSkillsCacheKey,
    ) -> Option<SkillLoadOutcome> {
        match self.cache_by_config.read() {
            Ok(cache) => cache.get(cache_key).cloned(),
            Err(err) => err.into_inner().get(cache_key).cloned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConfigSkillsCacheKey {
    roots: Vec<(AbsolutePathBuf, u8, Option<String>)>,
    skill_config_rules: SkillConfigRules,
    allowlist_patterns: Option<Vec<String>>,
}

fn config_skills_cache_key(
    roots: &[SkillRoot],
    skill_config_rules: &SkillConfigRules,
    allowlist_patterns: &Option<Vec<String>>,
) -> ConfigSkillsCacheKey {
    ConfigSkillsCacheKey {
        roots: roots
            .iter()
            .map(|root| {
                let scope_rank = match root.scope {
                    SkillScope::Repo => 0,
                    SkillScope::User => 1,
                    SkillScope::System => 2,
                    SkillScope::Admin => 3,
                };
                (root.path.clone(), scope_rank, root.plugin_id.clone())
            })
            .collect(),
        skill_config_rules: skill_config_rules.clone(),
        allowlist_patterns: allowlist_patterns.clone(),
    }
}

fn skill_matches_pattern(skill: &crate::SkillMetadata, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern == "*" {
        return true;
    }
    let namespaced_name = skill
        .plugin_id
        .as_ref()
        .map(|plugin_id| format!("{plugin_id}:{}", skill.name));
    if let Some(prefix) = pattern.strip_suffix('*') {
        skill.name.starts_with(prefix)
            || namespaced_name
                .as_deref()
                .is_some_and(|name| name.starts_with(prefix))
    } else {
        skill.name == pattern || namespaced_name.as_deref() == Some(pattern)
    }
}

fn finalize_skill_outcome(
    mut outcome: SkillLoadOutcome,
    disabled_paths: HashSet<AbsolutePathBuf>,
) -> SkillLoadOutcome {
    outcome.disabled_paths = disabled_paths;
    let (by_scripts_dir, by_doc_path, by_root_dir) =
        build_implicit_skill_path_indexes(outcome.allowed_skills_for_implicit_invocation());
    outcome.set_implicit_skill_indexes(by_scripts_dir, by_doc_path, by_root_dir);
    outcome
}

impl SkillsRuntime for SkillsManager {
    fn skills_for_config<'a>(
        &'a self,
        input: &'a SkillsLoadInput,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, SkillLoadOutcome> {
        Box::pin(async move { Self::skills_for_config(self, input, fs).await })
    }

    fn skills_for_cwd<'a>(
        &'a self,
        input: &'a SkillsLoadInput,
        force_reload: bool,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, SkillLoadOutcome> {
        Box::pin(async move { Self::skills_for_cwd(self, input, force_reload, fs).await })
    }

    fn skill_root_paths_for_config<'a>(
        &'a self,
        input: &'a SkillsLoadInput,
        fs: Option<Arc<dyn ExecutorFileSystem>>,
    ) -> SkillsRuntimeFuture<'a, Vec<AbsolutePathBuf>> {
        Box::pin(async move {
            Self::skill_roots_for_config(self, input, fs)
                .await
                .into_iter()
                .map(|root| root.path)
                .collect()
        })
    }

    fn clear_cache(&self) {
        Self::clear_cache(self);
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
