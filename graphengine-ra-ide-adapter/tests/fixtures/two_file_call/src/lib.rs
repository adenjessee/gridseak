//! Two-file fixture used by T6's `adapter_goto_definition_resolves_callee_from_main_rs`
//! integration test. Shape mirrors the spike fixture in §9.B2 so the
//! measured wall-clock baseline (135 ms `load_workspace_at`) stays
//! comparable.

pub fn callee() {
    println!("hello from callee");
}
