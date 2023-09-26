use containerd_shim_wamr::WAMRInstance;
use containerd_shim_wasm::sandbox::cli::{revision, shim_main, version};

fn main() {
    shim_main::<WAMRInstance>("wamr", version!(), revision!(), None);
}
