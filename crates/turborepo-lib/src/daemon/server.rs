//! Daemon Server
//!
//! This module houses the daemon server, some implementation notes for which
//! are below.
//!
//! ## Implementation Notes
//!
//! The basic goals of the daemon are to watch for, and be able to provide
//! details about, filesystem changes. It is organised as an async server, which
//! holds a `HashGlobWatcher` which holds data about hashes, globs to watch for
//! that hash, and files that have been updated for that hash. In addition, this
//! server can be interrogated over grpc to register interest in particular
//! globs, and to query for changes for those globs.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use globwatch::{StopSource, Watcher};
use tokio::{
    select,
    signal::ctrl_c,
    sync::{
        oneshot::{Receiver, Sender},
        Mutex,
    },
};
use tonic::transport::{NamedService, Server};
use turborepo_paths::{AbsoluteNormalizedPathBuf, ForwardRelativePath};

use super::{
    bump_timeout::BumpTimeout,
    proto::{self},
    DaemonError,
};
use crate::{commands::CommandBase, get_version, globwatcher::HashGlobWatcher};

pub struct DaemonServer<T: Watcher> {
    daemon_root: AbsoluteNormalizedPathBuf,
    log_file: AbsoluteNormalizedPathBuf,

    start_time: Instant,
    timeout: Duration,
    timeout_time: Arc<BumpTimeout>,

    watcher: Arc<HashGlobWatcher<T>>,
    shutdown: Mutex<Option<Sender<()>>>,
}

impl DaemonServer<notify::RecommendedWatcher> {
    pub fn new(
        base: &CommandBase,
        timeout: Duration,
        log_file: AbsoluteNormalizedPathBuf,
    ) -> Result<Self, DaemonError> {
        let daemon_root = base.daemon_file_root();

        let watcher = Arc::new(HashGlobWatcher::new(
            daemon_root
                .join(ForwardRelativePath::new("flush").expect("valid forward path"))
                .to_path_buf(),
        )?);

        Ok(Self {
            daemon_root,
            log_file,

            start_time: Instant::now(),
            timeout_time: Arc::new(BumpTimeout::new(timeout)),
            timeout,

            watcher,
            shutdown: Mutex::new(None),
        })
    }
}

impl<T: Watcher + Send + 'static> DaemonServer<T> {
    fn with_shutdown(mut self) -> (Self, Receiver<()>) {
        let (send_shutdown, recv_shutdown) = tokio::sync::oneshot::channel::<()>();
        self.shutdown = Mutex::new(Some(send_shutdown));
        (self, recv_shutdown)
    }

    /// Serve the daemon server, while also watching for filesystem changes.
    pub async fn serve(self, repo_root: AbsoluteNormalizedPathBuf) {
        let (server, shutdown_fut) = self.with_shutdown();

        let stop = StopSource::new();
        let watcher = server.watcher.clone();
        let watcher_fut = watcher.watch(repo_root.to_path_buf(), stop.token());

        let timer = server.timeout_time.clone();
        let timeout_fut = timer.wait();

        #[cfg(feature = "http")]
        let server_fut = {
            // set up grpc reflection
            let efd = include_bytes!("file_descriptor_set.bin");
            let reflection = tonic_reflection::server::Builder::configure()
                .register_encoded_file_descriptor_set(efd)
                .build()
                .unwrap();

            Server::builder()
                .add_service(reflection)
                .add_service(crate::daemon::proto::turbod_server::TurbodServer::new(
                    server,
                ))
                .serve_with_shutdown("127.0.0.1:5000".parse().unwrap(), shutdown_fut)
        };

        #[cfg(not(feature = "http"))]
        let (_lock, server_fut) = {
            let (lock, stream) = crate::daemon::endpoint::open_socket(server.daemon_root.clone())
                .await
                .unwrap();

            (
                lock,
                Server::builder()
                    .add_service(crate::daemon::proto::turbod_server::TurbodServer::new(
                        server,
                    ))
                    .serve_with_incoming(stream),
            )
        };

        select! {
            _ = server_fut => {}
            _ = watcher_fut => {}
            _ = shutdown_fut => {}
            _ = ctrl_c() => {}
            _ = timeout_fut => {}
        };

        // here the stop token is dropped, and the pid lock is dropped
        // causing them to be cleaned up
    }

    fn bump_timeout(&self) {
        self.timeout_time.reset(self.timeout);
    }
}

#[tonic::async_trait]
impl<T: Watcher + Send + 'static> proto::turbod_server::Turbod for DaemonServer<T> {
    async fn hello(
        &self,
        request: tonic::Request<proto::HelloRequest>,
    ) -> Result<tonic::Response<proto::HelloResponse>, tonic::Status> {
        self.bump_timeout();

        if request.into_inner().version != get_version() {
            return Err(tonic::Status::unimplemented("version mismatch"));
        } else {
            Ok(tonic::Response::new(proto::HelloResponse {}))
        }
    }

    async fn shutdown(
        &self,
        _request: tonic::Request<proto::ShutdownRequest>,
    ) -> Result<tonic::Response<proto::ShutdownResponse>, tonic::Status> {
        self.shutdown.lock().await.take().map(|s| s.send(()));

        // if Some(Ok), then the server is shutting down now
        // if Some(Err), then the server is already shutting down
        // if None, then someone has already called shutdown
        Ok(tonic::Response::new(proto::ShutdownResponse {}))
    }

    async fn status(
        &self,
        _request: tonic::Request<proto::StatusRequest>,
    ) -> Result<tonic::Response<proto::StatusResponse>, tonic::Status> {
        self.bump_timeout();

        Ok(tonic::Response::new(proto::StatusResponse {
            daemon_status: Some(proto::DaemonStatus {
                uptime_msec: self.start_time.elapsed().as_millis() as u64,
                log_file: self.log_file.to_str().unwrap().to_string(),
            }),
        }))
    }

    async fn notify_outputs_written(
        &self,
        request: tonic::Request<proto::NotifyOutputsWrittenRequest>,
    ) -> Result<tonic::Response<proto::NotifyOutputsWrittenResponse>, tonic::Status> {
        self.bump_timeout();

        let inner = request.into_inner();
        self.watcher
            .watch_globs(inner.hash, inner.output_globs, inner.output_exclusion_globs)
            .await;

        Ok(tonic::Response::new(proto::NotifyOutputsWrittenResponse {}))
    }

    async fn get_changed_outputs(
        &self,
        request: tonic::Request<proto::GetChangedOutputsRequest>,
    ) -> Result<tonic::Response<proto::GetChangedOutputsResponse>, tonic::Status> {
        self.bump_timeout();

        let inner = request.into_inner();
        let changed = self
            .watcher
            .changed_globs(&inner.hash, HashSet::from_iter(inner.output_globs))
            .await;

        Ok(tonic::Response::new(proto::GetChangedOutputsResponse {
            changed_output_globs: changed.into_iter().collect(),
        }))
    }
}

impl<T: Watcher> NamedService for DaemonServer<T> {
    const NAME: &'static str = "turborepo.Daemon";
}
