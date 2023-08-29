use std::path::PathBuf;

use containerd_shim_wasm::sandbox::oci::{self, Spec};
use containerd_shim_wasm::sandbox::Stdio;
use libcontainer::workload::{Executor, ExecutorError};
use wasmer::{Cranelift, Instance, Module, Store};
use wasmer_wasix::WasiEnv;

use crate::oci_wasmer;

const EXECUTOR_NAME: &str = "wasmer";

pub struct WasmerExecutor {
    stdio: Stdio,
    engine: Cranelift,
}

impl WasmerExecutor {
    pub fn new(stdio: Stdio, engine: Cranelift) -> Self {
        Self { stdio, engine }
    }
}

impl Executor for WasmerExecutor {
    fn name(&self) -> &'static str {
        EXECUTOR_NAME
    }

    fn exec(&self, spec: &containerd_shim_wasm::sandbox::oci::Spec) -> Result<(), ExecutorError> {
        log::info!("wasmer executor exec method");

        let args = oci::get_args(spec);
        if args.is_empty() {
            return Err(ExecutorError::InvalidArg);
        }

        self.start(spec, args)
            .map_err(|err| ExecutorError::Other(format!("failed to start wasm: {}", err)))?;

        std::process::exit(0)
    }

    fn can_handle(&self, spec: &containerd_shim_wasm::sandbox::oci::Spec) -> bool {
        // check if the entrypoint of the spec is a wasm binary.
        let (module_name, _method) = oci::get_module(spec);
        let module_name = match module_name {
            Some(m) => m,
            None => {
                log::info!("wasmer cannot handle this workload, no arguments provided");
                return false;
            }
        };
        let path = PathBuf::from(module_name);

        // TODO: do we need to validate the wasm binary?
        // ```rust
        //   let bytes = std::fs::read(path).unwrap();
        //   wasmparser::validate(&bytes).is_ok()
        // ```

        path.extension()
            .map(|ext| ext.to_ascii_lowercase())
            .is_some_and(|ext| ext == "wasm" || ext == "wat")
    }
}

impl WasmerExecutor {
    fn start(&self, spec: &Spec, args: &[String]) -> anyhow::Result<()> {
        // already in the cgroup
        let envs = oci_wasmer::env_to_wasi(spec);
        log::info!("setting up wasi");

        self.stdio.redirect()?;

        log::info!("wasi context ready");
        let (module_name, method) = oci::get_module(spec);
        let module_name = match module_name {
            Some(m) => m,
            None => {
                return Err(anyhow::format_err!(
                    "no module provided, cannot load module from file within container"
                ))
            }
        };

        log::info!("loading module from file {} ", module_name);
        let mut store = Store::new(self.engine.clone());
        let module = Module::from_file(&store, module_name)?;

        log::info!("Starting `tokio` runtime...");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = runtime.enter();

        log::info!("Creating `WasiEnv`.... args: {:?}, envs: {:?}", args, envs);
        let mut wasi_env = WasiEnv::builder(EXECUTOR_NAME)
            .args(args[1..].to_vec())
            .envs(envs)
            .preopen_dir("/")?
            .finalize(&mut store)?;

        log::info!("Instantiating module with WASI imports...");
        let import_object = wasi_env.import_object(&mut store, &module)?;
        let instance = Instance::new(&mut store, &module, &import_object)?;

        wasi_env.initialize(&mut store, instance.clone())?;

        log::info!("Call WASI `_start` function...");
        let start_func = instance.exports.get_function(&method)?;
        start_func.call(&mut store, &[])?;

        wasi_env.cleanup(&mut store, None);

        Ok(())
    }
}
