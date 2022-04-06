use crate::common::resources::ResourceAllocation;
use crate::common::stablemap::ExtractKey;
use crate::messages::worker::ComputeTaskMsg;
use crate::worker::taskenv::TaskEnv;
use crate::{InstanceId, Priority, TaskId, WorkerId};
use std::time::Duration;

pub enum TaskState {
    Waiting(u32),
    Running(TaskEnv, ResourceAllocation),
}

pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    pub priority: (Priority, Priority),
    pub instance_id: InstanceId,

    pub resources: crate::common::resources::ResourceRequest,
    pub time_limit: Option<Duration>,
    pub n_outputs: u32,
    pub body: Vec<u8>,
    pub node_list: Vec<WorkerId>, // Filled in multi-node tasks; otherwise empty
}

impl Task {
    pub fn new(message: ComputeTaskMsg) -> Self {
        Self {
            id: message.id,
            priority: (message.user_priority, message.scheduler_priority),
            state: TaskState::Waiting(0),
            instance_id: message.instance_id,
            resources: message.resources,
            time_limit: message.time_limit,
            n_outputs: message.n_outputs,
            body: message.body,
            node_list: message.node_list,
        }
    }

    #[inline]
    pub fn is_waiting(&self) -> bool {
        matches!(self.state, TaskState::Waiting(_))
    }

    #[inline]
    pub fn is_ready(&self) -> bool {
        matches!(self.state, TaskState::Waiting(0))
    }

    #[inline]
    pub fn is_running(&self) -> bool {
        matches!(self.state, TaskState::Running(_, _))
    }

    pub fn resource_allocation(&self) -> Option<&ResourceAllocation> {
        match &self.state {
            TaskState::Running(_, a) => Some(a),
            TaskState::Waiting(_) => None,
        }
    }

    pub fn task_env_mut(&mut self) -> Option<&mut TaskEnv> {
        match self.state {
            TaskState::Running(ref mut env, _) => Some(env),
            _ => None,
        }
    }

    pub fn get_waiting(&self) -> u32 {
        match self.state {
            TaskState::Waiting(x) => x,
            _ => 0,
        }
    }

    pub fn decrease_waiting_count(&mut self) -> bool {
        match &mut self.state {
            TaskState::Waiting(ref mut x) => {
                assert!(*x > 0);
                *x -= 1;
                *x == 0
            }
            _ => unreachable!(),
        }
    }

    pub fn increase_waiting_count(&mut self) {
        match &mut self.state {
            TaskState::Waiting(ref mut x) => {
                *x += 1;
            }
            _ => unreachable!(),
        }
    }
}

impl ExtractKey<TaskId> for Task {
    #[inline]
    fn extract_key(&self) -> TaskId {
        self.id
    }
}
