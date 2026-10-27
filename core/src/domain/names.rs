/// Provider part of an exposed model name: the provider name with whitespace and
/// `/` folded to `-`, so `{provider}/{model}` stays one readable token.
pub fn provider_slug(provider_name: &str) -> String {
    let mut slug = String::new();
    for word in provider_name.split(|ch: char| ch.is_whitespace() || ch == '/') {
        if word.is_empty() {
            continue;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(word);
    }
    slug
}

/// Model name exposed to clients: `{provider}/{upstream_model}`.
pub fn exposed_name(provider_name: &str, upstream_model: &str) -> String {
    format!("{}/{}", provider_slug(provider_name), upstream_model.trim())
}
