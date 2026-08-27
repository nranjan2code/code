use vak_tui::data::ClientData;

#[test]
fn runtime_ui_exports_only_runtime_data_adapter() {
    let _ = std::mem::size_of::<ClientData>();
}
