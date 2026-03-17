use leptos::prelude::*;

use crate::dashboard::DashboardPage;

#[component]
pub fn App() -> impl IntoView {
    view! { <DashboardPage /> }
}

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!doctype html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <title>"gigi dashboard"</title>
                <link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>🎤</text></svg>" />
                <link rel="stylesheet" href="/styles.css" />
                <AutoReload options=options.clone() />
                <HydrationScripts options=options />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}
