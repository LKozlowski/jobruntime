pub mod tls;

pub mod grpc_jobruntime {
    tonic::include_proto!("jobruntime");
}
