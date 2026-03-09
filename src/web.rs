use std::sync::Arc;

use axum::{
    Router,
    response::{
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures_util::stream::Stream;
use leptos::prelude::*;
use leptos_axum::{generate_route_list, LeptosRoutes};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::{
    config::AppConfig,
    dashboard_app::App,
    serve::{AppState, DashboardEvent},
};

pub async fn run_server(
    state: Arc<AppState>,
    config: &AppConfig,
) -> anyhow::Result<()> {
    let leptos_options = LeptosOptions::builder()
        .output_name("gigi")
        .site_root("target/site")
        .site_pkg_dir("pkg")
        .site_addr(
            format!("{}:{}", config.dashboard.host, config.dashboard.port)
                .parse()
                .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], 8787))),
        )
        .build();

    let routes = generate_route_list(App);

    let state_for_context = Arc::clone(&state);
    let app: Router<()> = Router::new()
        .route("/api/events", get({
            let state = Arc::clone(&state);
            move || sse_events(state)
        }))
        .leptos_routes_with_context(
            &leptos_options,
            routes,
            move || {
                leptos::context::provide_context((*state_for_context).clone());
            },
            {
                let opts = leptos_options.clone();
                move || shell(opts.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    let listener =
        tokio::net::TcpListener::bind((config.dashboard.host.as_str(), config.dashboard.port))
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "Failed to bind HTTP server on {}:{}: {err}",
                    config.dashboard.host,
                    config.dashboard.port
                )
            })?;

    axum::serve(listener, app)
        .await
        .map_err(anyhow::Error::from)
}

async fn sse_events(
    state: Arc<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let json = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().event(event_name(&event)).data(json)))
        }
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn event_name(event: &DashboardEvent) -> &'static str {
    match event {
        DashboardEvent::PollComplete(_) => "poll_complete",
        DashboardEvent::ReviewComplete { .. } => "review_complete",
    }
}

fn shell(options: LeptosOptions) -> impl IntoView {
    use leptos_meta::*;

    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}
