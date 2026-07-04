use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use codex_utils_absolute_path::AbsolutePathBuf;
use jsonrpc_types::JSONRPCErrorError;
use tokio::io;

use crate::CopyOptions;
use crate::CreateDirectoryOptions;
use crate::ExecServerRuntimePaths;
use crate::ExecutorFileSystem;
use crate::FileMetadata;
use crate::FileSystemFuture;
use crate::FileSystemResult;
use crate::FileSystemSandboxContext;
use crate::ReadDirectoryEntry;
use crate::RemoveOptions;
use crate::fs_helper::FsHelperPayload;
use crate::fs_helper::FsHelperRequest;
use crate::fs_sandbox::FileSystemSandboxRunner;
use crate::protocol::FsCopyParams;
use crate::protocol::FsCreateDirectoryParams;
use crate::protocol::FsGetMetadataParams;
use crate::protocol::FsReadDirectoryParams;
use crate::protocol::FsReadFileParams;
use crate::protocol::FsRemoveParams;
use crate::protocol::FsWriteFileParams;

#[derive(Clone)]
pub struct SandboxedFileSystem {
    sandbox_runner: FileSystemSandboxRunner,
}

impl SandboxedFileSystem {
    pub fn new(runtime_paths: ExecServerRuntimePaths) -> Self {
        Self {
            sandbox_runner: FileSystemSandboxRunner::new(runtime_paths),
        }
    }

    async fn run_sandboxed(
        &self,
        sandbox: &FileSystemSandboxContext,
        request: FsHelperRequest,
    ) -> FileSystemResult<FsHelperPayload> {
        self.sandbox_runner
            .run(sandbox, request)
            .await
            .map_err(map_sandbox_error)
    }
}

impl ExecutorFileSystem for SandboxedFileSystem {
    fn read_file<'a>(
        &'a self,
        path: &'a AbsolutePathBuf,
        sandbox: Option<&'a FileSystemSandboxContext>,
    ) -> FileSystemFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let sandbox = require_platform_sandbox(sandbox)?;
            let response = self
                .run_sandboxed(
                    sandbox,
                    FsHelperRequest::ReadFile(FsReadFileParams {
                        path: path.clone(),
                        sandbox: None,
                    }),
                )
                .await?
                .expect_read_file()
                .map_err(map_sandbox_error)?;
            STANDARD.decode(response.data_base64).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("fs/readFile returned invalid base64 dataBase64: {err}"),
                )
            })
        })
    }

    fn write_file<'a>(
        &'a self,
        path: &'a AbsolutePathBuf,
        contents: Vec<u8>,
        sandbox: Option<&'a FileSystemSandboxContext>,
    ) -> FileSystemFuture<'a, ()> {
        Box::pin(async move {
            let sandbox = require_platform_sandbox(sandbox)?;
            self.run_sandboxed(
                sandbox,
                FsHelperRequest::WriteFile(FsWriteFileParams {
                    path: path.clone(),
                    data_base64: STANDARD.encode(contents),
                    sandbox: None,
                }),
            )
            .await?
            .expect_write_file()
            .map_err(map_sandbox_error)?;
            Ok(())
        })
    }

    fn create_directory<'a>(
        &'a self,
        path: &'a AbsolutePathBuf,
        options: CreateDirectoryOptions,
        sandbox: Option<&'a FileSystemSandboxContext>,
    ) -> FileSystemFuture<'a, ()> {
        Box::pin(async move {
            let sandbox = require_platform_sandbox(sandbox)?;
            self.run_sandboxed(
                sandbox,
                FsHelperRequest::CreateDirectory(FsCreateDirectoryParams {
                    path: path.clone(),
                    recursive: Some(options.recursive),
                    sandbox: None,
                }),
            )
            .await?
            .expect_create_directory()
            .map_err(map_sandbox_error)?;
            Ok(())
        })
    }

    fn get_metadata<'a>(
        &'a self,
        path: &'a AbsolutePathBuf,
        sandbox: Option<&'a FileSystemSandboxContext>,
    ) -> FileSystemFuture<'a, FileMetadata> {
        Box::pin(async move {
            let sandbox = require_platform_sandbox(sandbox)?;
            let response = self
                .run_sandboxed(
                    sandbox,
                    FsHelperRequest::GetMetadata(FsGetMetadataParams {
                        path: path.clone(),
                        sandbox: None,
                    }),
                )
                .await?
                .expect_get_metadata()
                .map_err(map_sandbox_error)?;
            Ok(FileMetadata {
                is_directory: response.is_directory,
                is_file: response.is_file,
                is_symlink: response.is_symlink,
                created_at_ms: response.created_at_ms,
                modified_at_ms: response.modified_at_ms,
            })
        })
    }

    fn read_directory<'a>(
        &'a self,
        path: &'a AbsolutePathBuf,
        sandbox: Option<&'a FileSystemSandboxContext>,
    ) -> FileSystemFuture<'a, Vec<ReadDirectoryEntry>> {
        Box::pin(async move {
            let sandbox = require_platform_sandbox(sandbox)?;
            let response = self
                .run_sandboxed(
                    sandbox,
                    FsHelperRequest::ReadDirectory(FsReadDirectoryParams {
                        path: path.clone(),
                        sandbox: None,
                    }),
                )
                .await?
                .expect_read_directory()
                .map_err(map_sandbox_error)?;
            Ok(response
                .entries
                .into_iter()
                .map(|entry| ReadDirectoryEntry {
                    file_name: entry.file_name,
                    is_directory: entry.is_directory,
                    is_file: entry.is_file,
                })
                .collect())
        })
    }

    fn remove<'a>(
        &'a self,
        path: &'a AbsolutePathBuf,
        remove_options: RemoveOptions,
        sandbox: Option<&'a FileSystemSandboxContext>,
    ) -> FileSystemFuture<'a, ()> {
        Box::pin(async move {
            let sandbox = require_platform_sandbox(sandbox)?;
            self.run_sandboxed(
                sandbox,
                FsHelperRequest::Remove(FsRemoveParams {
                    path: path.clone(),
                    recursive: Some(remove_options.recursive),
                    force: Some(remove_options.force),
                    sandbox: None,
                }),
            )
            .await?
            .expect_remove()
            .map_err(map_sandbox_error)?;
            Ok(())
        })
    }

    fn copy<'a>(
        &'a self,
        source_path: &'a AbsolutePathBuf,
        destination_path: &'a AbsolutePathBuf,
        options: CopyOptions,
        sandbox: Option<&'a FileSystemSandboxContext>,
    ) -> FileSystemFuture<'a, ()> {
        Box::pin(async move {
            let sandbox = require_platform_sandbox(sandbox)?;
            self.run_sandboxed(
                sandbox,
                FsHelperRequest::Copy(FsCopyParams {
                    source_path: source_path.clone(),
                    destination_path: destination_path.clone(),
                    recursive: options.recursive,
                    sandbox: None,
                }),
            )
            .await?
            .expect_copy()
            .map_err(map_sandbox_error)?;
            Ok(())
        })
    }
}

fn require_platform_sandbox(
    sandbox: Option<&FileSystemSandboxContext>,
) -> FileSystemResult<&FileSystemSandboxContext> {
    sandbox
        .filter(|sandbox| sandbox.should_run_in_sandbox())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "sandboxed filesystem operations require ReadOnly or WorkspaceWrite sandbox policy",
            )
        })
}

fn map_sandbox_error(error: JSONRPCErrorError) -> io::Error {
    match error.code {
        -32004 => io::Error::new(io::ErrorKind::NotFound, error.message),
        -32600 => io::Error::new(io::ErrorKind::InvalidInput, error.message),
        _ => io::Error::other(error.message),
    }
}
