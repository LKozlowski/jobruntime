pub mod limits;

use limits::{Cgroup, ResourceLimits};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Stdio};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use uuid::Uuid;

const RUNTIME_EVENT_ERROR_MSG: &str = "runtime event channel does not work";
const RUNTIME_CGROUP_NAME: &str = "jobruntime";
const SIGKILL: i32 = 9;

// it's hardcoded just for the purpose of this program to simualte authorization
const ADMIN_ROLE: &str = "admin";

pub type JobId = Uuid;
pub type Owner = String;
pub type RuntimeSender = UnboundedSender<RuntimeCommand>;
pub type LogSender = UnboundedSender<LogRecord>;
pub type StatusSender = oneshot::Sender<Result<JobStatus, RuntimeError>>;
pub type StopSender = oneshot::Sender<Result<(), RuntimeError>>;
pub type StartSender = oneshot::Sender<JobId>;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("unathorized")]
    Unauthorized,
    #[error("job does not exists")]
    JobDoesNotExists,
    #[error("io error")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub enum LogRecord {
    Stdout(String),
    Stderr(String),
}

#[derive(Default, Debug)]
pub struct JobRequest {
    pub owner: String,
    pub path: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum JobStatus {
    Pending,
    Running { pid: i32 },
    Finished { exit_code: i32 },
    Killed { signal: i32 },
}

// Events that JobRuntime knows how to process
#[derive(Debug)]
enum RuntimeEvent {
    JobExit { job: JobId, status: ExitStatus },
    JobKill { job: JobId },
    JobStart { job: JobId, pid: i32 },
    LogCreated { job: JobId, record: LogRecord },
}

// Representation of the client that is connected
// to the runtime and fetches stream of logs
#[derive(Debug)]
struct LogClient {
    id: Uuid,
    sender: LogSender,
}

impl Hash for LogClient {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.id.hash(state)
    }
}

impl PartialEq for LogClient {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for LogClient {}

impl LogClient {
    fn new(sender: LogSender) -> Self {
        Self {
            id: Uuid::new_v4(),
            sender,
        }
    }
}

struct Job {
    uuid: Uuid,
    owner: Owner,
    logs: Vec<LogRecord>,
    kill_switch: Option<tokio::sync::oneshot::Sender<()>>,
    status: JobStatus,
}

impl Job {
    fn new(owner: String) -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let (rx, tx) = tokio::sync::oneshot::channel();
        let instance = Self {
            uuid: Uuid::new_v4(),
            owner,
            logs: Vec::new(),
            kill_switch: Some(rx),
            status: JobStatus::Pending,
        };

        (instance, tx)
    }

    fn started(&mut self, pid: i32) {
        self.status = JobStatus::Running { pid };
    }

    fn killed(&mut self, signal: i32) {
        self.status = JobStatus::Killed { signal };
    }

    fn finished(&mut self, exit_code: i32) {
        self.status = JobStatus::Finished { exit_code }
    }
}

#[derive(Debug)]
pub enum RuntimeCommand {
    Start {
        owner: Owner,
        path: String,
        args: Vec<String>,
        sender: StartSender,
        limits: ResourceLimits,
    },
    Stop {
        job: JobId,
        owner: Owner,
        sender: StopSender,
    },
    Status {
        job: JobId,
        owner: Owner,
        sender: StatusSender,
    },
    FetchLogs {
        job: JobId,
        owner: Owner,
        sender: LogSender,
    },
}

// Job runtime that spawn processes and stores its logs
pub struct JobRuntime {
    jobs: HashMap<Uuid, Job>,
    peers: HashMap<Uuid, HashSet<LogClient>>,
    event_rx: UnboundedReceiver<RuntimeEvent>,
    event_tx: UnboundedSender<RuntimeEvent>,
    cmd_rx: UnboundedReceiver<RuntimeCommand>,
    cgroup: Option<Cgroup>,
}

impl JobRuntime {
    pub fn new() -> (Self, UnboundedSender<RuntimeCommand>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let runtime = Self {
            jobs: HashMap::new(),
            peers: HashMap::new(),
            event_tx,
            event_rx,
            cmd_rx,
            cgroup: None,
        };
        (runtime, cmd_tx)
    }

    pub fn enable_cgroups(mut self) -> Result<Self, RuntimeError> {
        let cgroup = Cgroup::new(RUNTIME_CGROUP_NAME)?;
        cgroup.enable_controllers()?;
        self.cgroup = Some(cgroup);
        Ok(self)
    }

    // main event loop that accepts commands and process them
    // only place that is able to mutate interal state
    pub async fn start(mut self) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(command) = self.cmd_rx.recv() => {
                        match command {
                            RuntimeCommand::Start{path, args, owner, sender, limits} => {
                                if let Err(_) = self.start_job(path, args, owner, sender, limits){
                                    log::error!("unable to send response back to client");
                                };
                            },
                            RuntimeCommand::Stop{ job, owner, sender} => {
                                if let Err(_) = sender.send(self.stop_job(job, owner)) {
                                    log::error!("unable to send response back to client for job {}", job);
                                };
                            },
                            RuntimeCommand::Status { job, owner, sender} => {
                                if let Err(_) = sender.send(self.send_status(job, owner)) {
                                    log::error!("unable to send response back to client for job {}", job);
                                };
                            },
                            RuntimeCommand::FetchLogs{job, owner, sender} => {
                                if let Err(_) = self.send_logs(job, owner, sender) {
                                    log::error!("unable to send response back to client for job {}", job);
                                };
                            },
                        }
                    },
                    Some(event) = self.event_rx.recv() => {
                        match event {
                            RuntimeEvent::JobExit { job, status} => {
                                if let Ok(job_instance) = self.get_job(job) {
                                    if let Some(exit_code) = status.code() {
                                        job_instance.finished(exit_code);
                                    };

                                    if let Some(signal) = status.signal() {
                                        job_instance.killed(signal as i32);

                                    };
                                }
                            },
                            RuntimeEvent::JobKill { job } => {
                                if let Ok(job_instance) = self.get_job(job) {
                                    job_instance.killed(SIGKILL);
                                }
                            },
                            RuntimeEvent::JobStart { job, pid } => {
                                if let Ok(job_instance) = self.get_job(job) {
                                    job_instance.started(pid);
                                }
                            },
                            RuntimeEvent::LogCreated { job , record } => {
                                self.store_logs(job, record);
                            },
                        };
                    },
                }
            }
        });
    }

    fn get_job(&mut self, job: JobId) -> Result<&mut Job, RuntimeError> {
        if let Some(job_instance) = self.jobs.get_mut(&job) {
            Ok(job_instance)
        } else {
            Err(RuntimeError::JobDoesNotExists)
        }
    }

    async fn handle_job(
        job: JobId,
        mut child: Child,
        mut kill_switch: oneshot::Receiver<()>,
        event_tx: UnboundedSender<RuntimeEvent>,
    ) {
        let stdout = BufReader::new(child.stdout.take().expect("whatever stdout error"));
        let stderr = BufReader::new(child.stderr.take().expect("whatever stdout error"));
        let mut stdout_lines = stdout.lines();
        let mut stderr_lines = stderr.lines();

        if let Some(pid) = child.id() {
            event_tx
                .send(RuntimeEvent::JobStart {
                    job,
                    pid: pid as i32,
                })
                .expect(RUNTIME_EVENT_ERROR_MSG);
        };

        loop {
            tokio::select! {
                Ok(Some(data)) = stdout_lines.next_line() => {
                    event_tx.send(RuntimeEvent::LogCreated { job, record: LogRecord::Stdout(data)} ).expect(RUNTIME_EVENT_ERROR_MSG);
                },
                Ok(Some(data)) = stderr_lines.next_line() => {
                    event_tx.send(RuntimeEvent::LogCreated { job, record: LogRecord::Stderr(data)} ).expect(RUNTIME_EVENT_ERROR_MSG);
                },
                Ok(status) = child.wait() => {
                    event_tx.send(RuntimeEvent::JobExit{ job, status} ).expect(RUNTIME_EVENT_ERROR_MSG);
                    break;
                },
                _ = &mut kill_switch => {
                    match child.kill().await {
                        Ok(_) => {
                            event_tx.send(RuntimeEvent::JobKill { job } ).expect(RUNTIME_EVENT_ERROR_MSG);
                        },
                        Err(err) => {
                            log::error!("unable to kill process for job: {} | {}", job, err);
                        }
                    }
                    break;
                }
            };
        }
    }

    fn check_access_permissions(&self, job: JobId, owner: Owner) -> bool {
        if let Some(job_instance) = self.jobs.get(&job) {
            if (owner == ADMIN_ROLE) | (job_instance.owner == owner) {
                return true;
            };
        }
        false
    }

    fn store_logs(&mut self, job: JobId, record: LogRecord) {
        if let Some(job_instance) = self.jobs.get_mut(&job) {
            if let Some(peers) = self.peers.get_mut(&job) {
                // send message and remove clients that has closed the channel
                peers.retain(|e| {
                    if let Err(_) = e.sender.send(record.clone()) {
                        false
                    } else {
                        true
                    }
                });
            };
            job_instance.logs.push(record);
        }
    }

    fn send_logs(
        &mut self,
        job: JobId,
        owner: Owner,
        sender: LogSender,
    ) -> Result<(), RuntimeError> {
        if !self.check_access_permissions(job, owner) {
            return Err(RuntimeError::Unauthorized);
        };

        let job_instance = self.get_job(job)?;
        for each in &job_instance.logs {
            if let Err(e) = sender.send(each.clone()) {
                log::error!("error while sending logs back to the clinet. {}", e);
                break;
            };
        }
        match job_instance.status {
            JobStatus::Pending | JobStatus::Running { .. } => {
                let peers = self.peers.entry(job).or_insert(HashSet::new());
                peers.insert(LogClient::new(sender.clone()));
            }
            JobStatus::Finished { .. } | JobStatus::Killed { .. } => {}
        }
        Ok(())
    }

    fn stop_job(&mut self, job: JobId, owner: Owner) -> Result<(), RuntimeError> {
        if !self.check_access_permissions(job, owner) {
            return Err(RuntimeError::Unauthorized);
        };
        if let Some(rx) = self.get_job(job)?.kill_switch.take() {
            rx.send(()).expect("killing already dead job");
        };
        Ok(())
    }

    fn send_status(&mut self, job: JobId, owner: Owner) -> Result<JobStatus, RuntimeError> {
        if !self.check_access_permissions(job, owner) {
            return Err(RuntimeError::Unauthorized);
        };
        Ok(self.get_job(job)?.status.clone())
    }

    fn start_job(
        &mut self,
        path: String,
        args: Vec<String>,
        owner: String,
        sender: StartSender,
        limits: ResourceLimits,
    ) -> Result<(), RuntimeError> {
        let (job, kill_switch) = Job::new(owner);
        let cmd = Command::new(path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()?;

        let ret = job.uuid.clone();

        if let Some(runtime_cgroup) = &self.cgroup {
            let job_cgroup = Cgroup::new_relative_to(&runtime_cgroup, &ret.to_string())?;
            job_cgroup.apply_limits(limits)?;
            job_cgroup.add_task(&cmd)?;
        };

        tokio::spawn(Self::handle_job(
            job.uuid.clone(),
            cmd,
            kill_switch,
            self.event_tx.clone(),
        ));

        self.jobs.insert(job.uuid, job);
        if let Err(_) = sender.send(ret) {
            log::error!("start_job can't send back response to client");
        };
        Ok(())
    }
}
