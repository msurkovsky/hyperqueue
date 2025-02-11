use std::ops::ControlFlow;
use std::{io, thread};

use tako::common::WrappedRcRefCell;
use termion::input::TermRead;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Duration;

use crate::client::globalsettings::GlobalSettings;
use crate::dashboard::data;
use crate::dashboard::data::DashboardData;
use crate::dashboard::events::DashboardEvent;
use crate::dashboard::ui::screens::root_screen::RootScreen;
use crate::dashboard::ui::terminal::initialize_terminal;
use crate::server::bootstrap::get_client_connection;

/// Starts the dashboard UI with a keyboard listener and tick provider
pub async fn start_ui_loop(gsettings: &GlobalSettings) -> anyhow::Result<()> {
    let connection = get_client_connection(gsettings.server_directory()).await?;

    // TODO: When we start the dashboard and connect to the server, the server may have already forgotten
    // some of its events. Therefore we should bootstrap the state with the most recent overview snapshot.
    let dashboard_data = WrappedRcRefCell::wrap(DashboardData::default());

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    start_key_event_listener(tx.clone());
    let mut root_screen = RootScreen::default();

    let ui_ticker = send_event_every(100, tx.clone(), DashboardEvent::UiTick);
    let data_fetch_process =
        data::create_data_fetch_process(Duration::from_secs(1), dashboard_data.clone(), connection);

    let mut terminal = initialize_terminal()?;

    let event_loop = async {
        loop {
            if let Some(dashboard_event) = rx.recv().await {
                match dashboard_event {
                    DashboardEvent::KeyPressEvent(input) => {
                        if let ControlFlow::Break(res) = root_screen.handle_key(input) {
                            break res;
                        }
                    }
                    DashboardEvent::UiTick => {
                        let data = dashboard_data.get();
                        root_screen.draw(&mut terminal, &data);
                    }
                }
            }
        }
    };

    tokio::select! {
        _ = ui_ticker => {
            log::warn!("UI event process has ended");
            Ok(())
        }
        result = data_fetch_process => {
            log::warn!("Data fetch process has ended");
            result
        }
        result = event_loop => {
            log::warn!("Dashboard event loop has ended");
            result
        }
    }
}

/// Handles key press events when the dashboard_ui is active
fn start_key_event_listener(tx: UnboundedSender<DashboardEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let stdin = io::stdin();
        for key in stdin.keys().flatten() {
            if let Err(err) = tx.send(DashboardEvent::KeyPressEvent(key)) {
                eprintln!("Error in sending dashboard key: {}", err);
                return;
            }
        }
    })
}

/// Sends a dashboard event every n milliseconds
async fn send_event_every(
    n_milliseconds: u64,
    sender: UnboundedSender<DashboardEvent>,
    event_type: DashboardEvent,
) {
    let mut tick_duration = tokio::time::interval(Duration::from_millis(n_milliseconds));
    loop {
        if let Err(e) = sender.send(event_type) {
            log::error!("Error in producing dashboard events: {}", e);
            return;
        }
        tick_duration.tick().await;
    }
}
