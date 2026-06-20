const DEFAULT_USER_FIRST_NAME: &str = "there";

pub fn current_user_first_name() -> String {
    current_user_first_name_with_default(DEFAULT_USER_FIRST_NAME)
}

pub fn current_user_first_name_with_default(default: &str) -> String {
    [whoami::realname(), whoami::username()]
        .into_iter()
        .filter_map(|name| name.split_whitespace().next().map(str::to_string))
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::current_user_first_name;
    use super::current_user_first_name_with_default;

    #[test]
    fn current_user_first_name_returns_non_empty_value() {
        assert!(!current_user_first_name().is_empty());
    }

    #[test]
    fn current_user_first_name_with_default_returns_non_empty_value() {
        assert!(!current_user_first_name_with_default("there").is_empty());
    }
}
