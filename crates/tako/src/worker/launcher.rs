use std::fs::File;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;

use crate::common::error::DsError::GenericError;
use bstr::ByteSlice;
use tokio::process::Command;

use crate::messages::common::{ProgramDefinition, StdioDef};
use crate::server::task::SerializedTaskContext;
use crate::worker::state::WorkerState;
use crate::worker::taskenv::{StopReason, TaskResult};
use crate::TaskId;

pub type TaskFuture = Pin<Box<dyn Future<Output = crate::Result<TaskResult>>>>;

pub struct TaskLaunchData {
    /// Future that represents the execution of the task
    pub task_future: TaskFuture,

    /// Opaque data that will be sent back to the server
    pub task_context: SerializedTaskContext,
}

impl TaskLaunchData {
    #[inline]
    pub fn new(task_future: TaskFuture, task_context: SerializedTaskContext) -> Self {
        Self {
            task_future,
            task_context,
        }
    }

    #[inline]
    pub fn from_future(task_future: TaskFuture) -> Self {
        Self {
            task_future,
            task_context: Default::default(),
        }
    }
}

pub trait TaskLauncher {
    /// Creates data required to execute a task.
    /// `state_ref` must not be borrowed when this function is called.  
    fn build_task(
        &self,
        state: &WorkerState,
        task_id: TaskId,
        stop_receiver: tokio::sync::oneshot::Receiver<StopReason>,
    ) -> crate::Result<TaskLaunchData>;
}

/// Create an output stream file on the given path.
/// If the path is relative, the file will be created relative to `cwd`.
fn create_output_stream(def: &StdioDef, cwd: &Path) -> crate::Result<Stdio> {
    let stdio = match def {
        StdioDef::File(path) => {
            let stream_path = if path.is_relative() {
                cwd.join(path)
            } else {
                path.clone()
            };

            let file = File::create(stream_path)
                .map_err(|e| format!("Creating stream file failed: {}", e))?;
            Stdio::from(file)
        }
        StdioDef::Null => Stdio::null(),
        StdioDef::Pipe => Stdio::piped(),
    };
    Ok(stdio)
}

pub fn command_from_definitions(definition: &ProgramDefinition) -> crate::Result<Command> {
    if definition.args.is_empty() {
        return Result::Err(crate::Error::GenericError(
            "No command arguments".to_string(),
        ));
    }

    let mut command = Command::new(definition.args[0].to_os_str_lossy());

    command.kill_on_drop(true);
    command.args(definition.args[1..].iter().map(|x| x.to_os_str_lossy()));

    std::fs::create_dir_all(&definition.cwd).map_err(|error| {
        GenericError(format!("Could not create working directory: {:?}", error))
    })?;
    command.current_dir(&definition.cwd);

    command.stdout(create_output_stream(&definition.stdout, &definition.cwd)?);
    command.stderr(create_output_stream(&definition.stderr, &definition.cwd)?);

    command.stdin(if definition.stdin.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    });

    for (k, v) in definition.env.iter() {
        command.env(k.to_os_str_lossy(), v.to_os_str_lossy());
    }

    Ok(command)
}
