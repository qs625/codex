use super::*;

fn load_catalog_json(path: &AbsolutePathBuf) -> std::io::Result<ModelsResponse> {
    let file_contents = std::fs::read_to_string(path)?;
    let catalog = serde_json::from_str::<ModelsResponse>(&file_contents).map_err(|err| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "failed to parse model_catalog_json path `{}` as JSON: {err}",
                path.display()
            ),
        )
    })?;
    if catalog.models.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "model_catalog_json path `{}` must contain at least one model",
                path.display()
            ),
        ));
    }
    Ok(catalog)
}

pub(super) fn load_model_catalog(
    model_catalog_json: Option<AbsolutePathBuf>,
) -> std::io::Result<Option<ModelsResponse>> {
    model_catalog_json
        .map(|path| load_catalog_json(&path))
        .transpose()
}

pub(super) fn validate_model_options(model_options: &[ModelOptionToml]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for model_option in model_options {
        if model_option.provider.trim().is_empty() {
            return Err("model_options.provider must not be empty".to_string());
        }
        if model_option.model.trim().is_empty() {
            return Err(format!(
                "model_options.{}: model must not be empty",
                model_option.provider
            ));
        }
        let key = (&model_option.provider, &model_option.model);
        if !seen.insert(key) {
            return Err(format!(
                "model_options contains duplicate provider/model pair: {}/{}",
                model_option.provider, model_option.model
            ));
        }
        if model_option.context_window.is_some_and(|value| value <= 0) {
            return Err(format!(
                "model_options.{}/{}: context_window must be positive",
                model_option.provider, model_option.model
            ));
        }
        if model_option
            .max_context_window
            .is_some_and(|value| value <= 0)
        {
            return Err(format!(
                "model_options.{}/{}: max_context_window must be positive",
                model_option.provider, model_option.model
            ));
        }
        if model_option
            .auto_compact_token_limit
            .is_some_and(|value| value <= 0)
        {
            return Err(format!(
                "model_options.{}/{}: auto_compact_token_limit must be positive",
                model_option.provider, model_option.model
            ));
        }
    }
    Ok(())
}
