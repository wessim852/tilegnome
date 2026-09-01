use std::{error::Error, sync::Mutex};

use dwindle_core::{Command, EngineState, Response};
use tracing::{error, info};

const BUS_NAME: &str = "dev.dwindlers.Engine";
const OBJECT_PATH: &str = "/dev/dwindlers/Engine";

#[derive(Default)]
struct EngineService {
    state: Mutex<EngineState>,
}

#[zbus::interface(name = "dev.dwindlers.Engine")]
impl EngineService {
    fn request(&self, json: &str) -> String {
        let response = match serde_json::from_str::<Command>(json) {
            Ok(command) => match self.state.lock() {
                Ok(mut state) => state.handle(command),
                Err(error) => {
                    error!(event = "ERROR", error = %error, "engine state lock poisoned");
                    Response::Error {
                        message: "engine state is unavailable".into(),
                    }
                }
            },
            Err(error) => {
                error!(event = "ERROR", error = %error, "invalid D-Bus request JSON");
                Response::Error {
                    message: format!("invalid request: {error}"),
                }
            }
        };
        serde_json::to_string(&response).unwrap_or_else(|error| {
            error!(event = "ERROR", error = %error, "response serialization failed");
            r#"{"type":"error","message":"response serialization failed"}"#.into()
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dwindle_daemon=info,dwindle_core=info".into()),
        )
        .with_target(false)
        .init();

    let connection = zbus::Connection::session().await?;
    connection.request_name(BUS_NAME).await?;
    connection
        .object_server()
        .at(OBJECT_PATH, EngineService::default())
        .await?;
    info!(
        event = "DAEMON_READY",
        bus_name = BUS_NAME,
        object_path = OBJECT_PATH
    );
    tokio::signal::ctrl_c().await?;
    info!(event = "DAEMON_STOP");
    Ok(())
}
