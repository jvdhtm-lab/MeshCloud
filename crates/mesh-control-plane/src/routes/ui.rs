use axum::response::Html;

/// Serve the single-page dashboard.
pub async fn index() -> Html<&'static str> {
    Html(include_str!("../ui.html"))
}
