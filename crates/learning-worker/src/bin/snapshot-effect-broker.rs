//! Optional Root controller companion. S1 deliberately has no DB configuration
//! or writer switch. It never starts as part of the legacy worker mode.
#[cfg(target_os = "linux")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use learning_worker::snapshot_broker::DisabledBroker;
    use std::{env, io, path::Path};
    let args: Vec<String> = env::args().skip(1).collect();
    let [mode, root_flag, root, ipc_flag, ipc, epoch_flag, epoch] = args.as_slice() else {
        return Err(io::Error::other(
            "expected --serve-disabled --custody-root ABS --ipc-parent ABS --epoch UUID",
        )
        .into());
    };
    if mode != "--serve-disabled"
        || root_flag != "--custody-root"
        || ipc_flag != "--ipc-parent"
        || epoch_flag != "--epoch"
    {
        return Err(io::Error::other("invalid broker arguments").into());
    }
    let id: uuid::Uuid = epoch.parse()?;
    if id.is_nil() || id.to_string() != *epoch {
        return Err(io::Error::other("invalid broker epoch").into());
    }
    let broker = DisabledBroker::bind(Path::new(root), Path::new(ipc), id)?;
    eprintln!("snapshot broker ready: writer disabled, effects Unknown, no database");
    broker.serve_disabled()?;
    Ok(())
}

fn main() {
    #[cfg(target_os = "linux")]
    let result = run();
    #[cfg(not(target_os = "linux"))]
    let result: Result<(), Box<dyn std::error::Error>> =
        Err("snapshot broker requires Linux".into());
    if result.is_err() {
        eprintln!("snapshot broker unavailable; no database or writer fallback");
        std::process::exit(78);
    }
}
