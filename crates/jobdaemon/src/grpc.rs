use crate::tls;
use bytes::Bytes;
use futures::Stream;
use grpc_jobruntime::job_runtime_server::JobRuntime;
use grpc_jobruntime::{
    job_status_response, JobLogsRequest, JobLogsResponse, JobResourceLimits, JobStartRequest,
    JobStartResponse, JobStatusRequest, JobStatusResponse, JobStopRequest, JobStopResponse,
};
use runtime::{limits::ResourceLimits, JobStatus, LogRecord, RuntimeCommand, RuntimeSender};
use std::pin::Pin;
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub mod grpc_jobruntime {
    tonic::include_proto!("jobruntime");
}

pub type Username = String;

pub struct MyJobRuntime {
    cmd_tx: RuntimeSender,
}

impl MyJobRuntime {
    pub fn new(cmd_tx: RuntimeSender) -> Self {
        Self { cmd_tx }
    }
}

// TODO: check it
impl From<Bytes> for JobLogsResponse {
    fn from(data: Bytes) -> Self {
        JobLogsResponse {
            data: data.to_vec(),
        }
    }
}

impl From<&JobResourceLimits> for ResourceLimits {
    fn from(limits: &JobResourceLimits) -> Self {
        let mut ret = ResourceLimits::default();

        if limits.memory_high > 0 {
            ret.memory_high = Some(limits.memory_high)
        }

        if limits.memory_max > 0 {
            ret.memory_max = Some(limits.memory_max)
        }

        if limits.cpu_weight > 0 {
            ret.cpu_weight = Some(limits.cpu_weight)
        }

        if limits.cpu_max > 0 {
            ret.cpu_max = Some(limits.cpu_max)
        }

        if limits.io_weight > 0 {
            ret.io_weight = Some(limits.io_weight)
        }

        ret
    }
}

fn extract_username_from_request<T>(request: &Request<T>) -> Result<Username, Status> {
    match request.extensions().get::<tls::UsernameExtension>() {
        Some(extension) => Ok(String::from(&extension.username)),
        None => Err(Status::internal("unable to get username extension")),
    }
}

#[tonic::async_trait]
impl JobRuntime for MyJobRuntime {
    async fn start_job(
        &self,
        request: Request<JobStartRequest>,
    ) -> Result<Response<JobStartResponse>, Status> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let username = extract_username_from_request(&request)?;

        let limits = match &request.get_ref().limits {
            Some(limits) => limits.into(),
            None => ResourceLimits::default(),
        };

        let mut args = request.into_inner().args;
        let (path, args) = if args.len() >= 2 {
            let (left, right) = args.split_at_mut(1);
            (String::from(left[0].clone()), Vec::from(right))
        } else if args.len() == 1 {
            (String::from(args[0].clone()), vec![])
        } else {
            return Err(Status::invalid_argument("args must be greater that 0"));
        };

        let cmd = RuntimeCommand::Start {
            path,
            args,
            owner: username,
            sender: tx,
            limits,
        };

        if let Err(_) = self.cmd_tx.send(cmd) {
            return Err(Status::internal("runtime command channel error"));
        };

        let result = rx
            .await
            .map_err(|err| Status::unknown(format!("connection error: {}", err)))?;

        Ok(Response::new(JobStartResponse {
            uuid: result.to_simple().to_string(),
        }))
    }

    type FetchJobLogsStream = Pin<Box<dyn Stream<Item = Result<JobLogsResponse, Status>> + Send>>;

    async fn fetch_job_logs(
        &self,
        request: Request<JobLogsRequest>,
    ) -> Result<Response<Self::FetchJobLogsStream>, Status> {
        let (sender, rx) = tokio::sync::mpsc::unbounded_channel();
        let owner = extract_username_from_request(&request)?;
        let job = Uuid::parse_str(&request.get_ref().uuid).map_err(|_| {
            Status::invalid_argument(format!("invalid uuid: {}", request.get_ref().uuid))
        })?;
        let cmd = RuntimeCommand::FetchLogs { job, sender, owner };

        if let Err(_) = self.cmd_tx.send(cmd) {
            return Err(Status::internal("runtime command channel error"));
        };

        let log_receiver = UnboundedReceiverStream::new(rx);
        let log_stream = log_receiver.map(|item| {
            let resp = JobLogsResponse {
                data: match item {
                    LogRecord::Stderr(data) | LogRecord::Stdout(data) => Bytes::from(data).to_vec(),
                },
            };
            Ok(resp)
        });
        Ok(Response::new(Box::pin(log_stream)))
    }

    async fn stop_job(
        &self,
        request: Request<JobStopRequest>,
    ) -> Result<Response<JobStopResponse>, Status> {
        let (sender, rx) = tokio::sync::oneshot::channel();
        let job = Uuid::parse_str(&request.get_ref().uuid).map_err(|_| {
            Status::invalid_argument(format!("invalid uuid: {}", request.get_ref().uuid))
        })?;
        let owner = extract_username_from_request(&request)?;
        let cmd = RuntimeCommand::Stop { job, owner, sender };

        if let Err(_) = self.cmd_tx.send(cmd) {
            return Err(Status::internal("runtime command channel error"));
        };

        rx.await
            .map_err(|err| Status::unknown(format!("connection error: {}", err)))?
            .map_err(|err| Status::unknown(format!("runtime error: {}", err)))?;

        Ok(Response::new(JobStopResponse {}))
    }

    async fn fetch_job_status(
        &self,
        request: Request<JobStatusRequest>,
    ) -> Result<Response<JobStatusResponse>, Status> {
        let (sender, rx) = tokio::sync::oneshot::channel();
        let owner = extract_username_from_request(&request)?;

        let job = Uuid::parse_str(&request.into_inner().uuid)
            .or_else(|_| Err(Status::invalid_argument("invalid uuid")))?;

        let cmd = RuntimeCommand::Status { job, sender, owner };

        if let Err(_) = self.cmd_tx.send(cmd) {
            return Err(Status::internal("runtime command channel error"));
        };

        let result = rx
            .await
            .map_err(|err| Status::unknown(format!("connection error: {}", err)))?
            .map_err(|err| Status::unknown(format!("runtime error: {}", err)))?;

        let status = match result.status {
            JobStatus::Finished { exit_code } => {
                Some(job_status_response::Status::ExitCode(exit_code))
            }
            JobStatus::Running { pid } => Some(job_status_response::Status::Pid(pid)),
            JobStatus::Killed { signal } => Some(job_status_response::Status::Signal(signal)),
            JobStatus::Pending => None,
        };

        Ok(Response::new(JobStatusResponse {
            status,
            uuid: result.job.to_simple().to_string(),
            owner: result.owner,
        }))
    }
}
