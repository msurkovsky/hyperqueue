use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tako::common::resources::ResourceDescriptor;
use tako::common::Map;
use tako::messages::common::WorkerConfiguration;
use tempdir::TempDir;

use tako::worker::state::ServerLostPolicy;

use crate::client::globalsettings::GlobalSettings;
use crate::common::manager::info::{ManagerInfo, WORKER_EXTRA_MANAGER_KEY};
use crate::common::utils::network::get_hostname;
use crate::common::utils::time::ArgDuration;
use crate::rpc_call;
use crate::transfer::connection::ClientConnection;
use crate::transfer::messages::{
    FromClientMessage, IdSelector, StopWorkerMessage, StopWorkerResponse, ToClientMessage,
    WorkerInfo, WorkerInfoRequest,
};
use crate::worker::bootstrap::{
    finalize_configuration, initialize_worker, try_get_pbs_info, try_get_slurm_info,
};
use crate::worker::hwdetect::{detect_cpus, detect_cpus_no_ht, detect_generic_resources};
use crate::worker::parser::{ArgCpuDefinition, ArgGenericResourceDef, CpuDefinition};
use crate::WorkerId;

#[derive(clap::ArgEnum, Clone)]
pub enum WorkerFilter {
    Running,
    Offline,
}

#[derive(clap::ArgEnum, Clone)]
pub enum ManagerOpts {
    Detect,
    None,
    Pbs,
    Slurm,
}

#[derive(clap::ArgEnum, Clone)]
pub enum ArgServerLostPolicy {
    Stop,
    FinishRunning,
}

impl From<ArgServerLostPolicy> for ServerLostPolicy {
    fn from(policy: ArgServerLostPolicy) -> Self {
        match policy {
            ArgServerLostPolicy::Stop => ServerLostPolicy::Stop,
            ArgServerLostPolicy::FinishRunning => ServerLostPolicy::FinishRunning,
        }
    }
}

#[derive(Parser)]
pub struct WorkerStartOpts {
    /// How many cores should be allocated for the worker
    #[clap(long, default_value = "auto")]
    pub cpus: ArgCpuDefinition,

    /// Resources
    #[clap(long, multiple_occurrences(true))]
    pub resource: Vec<ArgGenericResourceDef>,

    #[clap(long = "no-detect-resources")]
    /// Disable auto-detection of resources
    pub no_detect_resources: bool,

    /// How often should the worker announce its existence to the server. (default: "8s")
    #[clap(long, default_value = "8s")]
    pub heartbeat: ArgDuration,

    /// Duration after which will an idle worker automatically stop
    #[clap(long)]
    pub idle_timeout: Option<ArgDuration>,

    /// Worker time limit. Worker exits after given time.
    #[clap(long)]
    pub time_limit: Option<ArgDuration>,

    /// What HPC job manager should be used by the worker.
    #[clap(long, default_value = "detect", arg_enum)]
    pub manager: ManagerOpts,

    /// Overwrite worker hostname
    #[clap(long)]
    pub hostname: Option<String>,

    /// Behavior when a connection to a server is lost
    #[clap(long, default_value = "stop", arg_enum)]
    pub on_server_lost: ArgServerLostPolicy,

    /// Working directory of a worker. Temp directory by default.
    /// It should *NOT* be placed on a network filesystem.
    #[clap(long)]
    pub work_dir: Option<PathBuf>,
}

pub async fn start_hq_worker(
    gsettings: &GlobalSettings,
    opts: WorkerStartOpts,
) -> anyhow::Result<()> {
    let mut configuration = gather_configuration(opts)?;
    finalize_configuration(&mut configuration);

    let (future, worker) = initialize_worker(gsettings.server_directory(), configuration).await?;

    gsettings.printer().print_worker_info(WorkerInfo {
        id: worker.id,
        configuration: worker.configuration,
        ended: None,
    });
    future.await.map_err(|e| e.into())
}

fn gather_configuration(opts: WorkerStartOpts) -> anyhow::Result<WorkerConfiguration> {
    log::debug!("Gathering worker configuration information");

    let hostname = get_hostname(opts.hostname);

    let cpus = match opts.cpus.unpack() {
        CpuDefinition::Detect => detect_cpus()?,
        CpuDefinition::DetectNoHyperThreading => detect_cpus_no_ht()?,
        CpuDefinition::Custom(cpus) => cpus,
    };

    let mut generic = if opts.no_detect_resources {
        Vec::new()
    } else {
        detect_generic_resources()?
    };
    for def in opts.resource {
        let descriptor = def.unpack();
        generic.retain(|desc| desc.name != descriptor.name);
        generic.push(descriptor)
    }

    let resources = ResourceDescriptor::new(cpus, generic);
    resources.validate()?;

    let (work_dir, log_dir) = {
        let tmpdir = TempDir::new("hq-worker").unwrap().into_path();
        (
            opts.work_dir.unwrap_or_else(|| tmpdir.join("work")),
            tmpdir.join("logs"),
        )
    };

    let manager_info = gather_manager_info(opts.manager)?;
    let mut extra = Map::new();

    if let Some(manager_info) = &manager_info {
        extra.insert(
            WORKER_EXTRA_MANAGER_KEY.to_string(),
            serde_json::to_string(&manager_info)?,
        );
    }

    Ok(WorkerConfiguration {
        resources,
        listen_address: Default::default(), // Will be filled during init
        time_limit: opts
            .time_limit
            .map(|x| x.unpack())
            .or_else(|| manager_info.and_then(|m| m.time_limit)),
        hostname,
        work_dir,
        log_dir,
        on_server_lost: opts.on_server_lost.into(),
        heartbeat_interval: opts.heartbeat.unpack(),
        idle_timeout: opts.idle_timeout.map(|x| x.unpack()),
        send_overview_interval: Some(Duration::from_millis(1000)),
        extra,
    })
}

fn gather_manager_info(opts: ManagerOpts) -> anyhow::Result<Option<ManagerInfo>> {
    match opts {
        ManagerOpts::Detect => {
            log::debug!("Trying to detect manager");
            Ok(try_get_pbs_info().or_else(|_| try_get_slurm_info()).ok())
        }
        ManagerOpts::None => {
            log::debug!("Manager detection disabled");
            Ok(None)
        }
        ManagerOpts::Pbs => Ok(Some(try_get_pbs_info()?)),
        ManagerOpts::Slurm => Ok(Some(try_get_slurm_info()?)),
    }
}

pub async fn get_worker_list(
    connection: &mut ClientConnection,
    filter: Option<WorkerFilter>,
) -> crate::Result<Vec<WorkerInfo>> {
    let msg = rpc_call!(
        connection,
        FromClientMessage::WorkerList,
        ToClientMessage::WorkerListResponse(r) => r
    )
    .await?;

    let mut workers = msg.workers;

    if let Some(filter) = filter {
        match filter {
            WorkerFilter::Running => workers.retain(|w| w.ended.is_none()),
            WorkerFilter::Offline => workers.retain(|w| w.ended.is_some()),
        };
    }

    workers.sort_unstable_by_key(|w| w.id);
    Ok(workers)
}

pub async fn get_worker_info(
    connection: &mut ClientConnection,
    worker_id: WorkerId,
) -> crate::Result<Option<WorkerInfo>> {
    let msg = rpc_call!(
        connection,
        FromClientMessage::WorkerInfo(WorkerInfoRequest {
            worker_id,
        }),
        ToClientMessage::WorkerInfoResponse(r) => r
    )
    .await?;

    Ok(msg)
}

pub async fn stop_worker(
    connection: &mut ClientConnection,
    selector: IdSelector,
) -> crate::Result<()> {
    let message = FromClientMessage::StopWorker(StopWorkerMessage { selector });
    let mut responses =
        rpc_call!(connection, message, ToClientMessage::StopWorkerResponse(r) => r).await?;

    responses.sort_unstable_by_key(|x| x.0);
    for (id, response) in responses {
        match response {
            StopWorkerResponse::Failed(e) => {
                log::error!("Stopping worker {} failed; {}", id, e.to_string());
            }
            StopWorkerResponse::InvalidWorker => {
                log::error!("Stopping worker {} failed; worker not found", id);
            }
            StopWorkerResponse::AlreadyStopped => {
                log::warn!("Stopping worker {} failed; worker is already stopped", id);
            }
            StopWorkerResponse::Stopped => {
                log::info!("Worker {} stopped", id)
            }
        }
    }

    Ok(())
}
