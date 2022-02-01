use crate::client::status::Status;
use crate::common::arraydef::IntArray;
use crate::transfer::messages::{IdSelector, TaskIdSelector, TaskSelector, TaskStatusSelector};
use clap::Parser;
use std::str::FromStr;

pub enum IdSelectorArg {
    All,
    Last,
    Id(IntArray),
}

impl FromStr for IdSelectorArg {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "last" => Ok(IdSelectorArg::Last),
            "all" => Ok(IdSelectorArg::All),
            _ => Ok(IdSelectorArg::Id(IntArray::from_str(s)?)),
        }
    }
}

impl From<IdSelectorArg> for IdSelector {
    fn from(id_selector_arg: IdSelectorArg) -> Self {
        match id_selector_arg {
            IdSelectorArg::Id(array) => IdSelector::Specific(array),
            IdSelectorArg::Last => IdSelector::LastN(1),
            IdSelectorArg::All => IdSelector::All,
        }
    }
}

#[derive(Parser)]
pub struct TaskSelectorArg {
    /// Filter task(s) by ID.
    #[clap(long)]
    pub tasks: Option<IntArray>,

    /// Filter task(s) by status.
    /// You can use multiple states separated by a comma.
    #[clap(long, multiple_occurrences(false), use_delimiter(true), arg_enum)]
    pub task_status: Vec<Status>,
}

pub fn get_task_selector(opt_task_selector_arg: Option<TaskSelectorArg>) -> Option<TaskSelector> {
    opt_task_selector_arg.map(|arg| TaskSelector {
        id_selector: get_id_selector(arg.tasks),
        status_selector: get_status_selector(arg.task_status),
    })
}

fn get_id_selector(id_selector_arg: Option<IntArray>) -> TaskIdSelector {
    id_selector_arg
        .map(TaskIdSelector::Specific)
        .unwrap_or(TaskIdSelector::All)
}

fn get_status_selector(status_selector_arg: Vec<Status>) -> TaskStatusSelector {
    if status_selector_arg.is_empty() {
        TaskStatusSelector::All
    } else {
        TaskStatusSelector::Specific(status_selector_arg)
    }
}
