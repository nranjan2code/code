use tokio::io::AsyncReadExt;

#[tokio::test]
async fn persistent_preview_crosses_the_versioned_worker_boundary() {
    let root = tempfile::tempdir().expect("temporary preview root");
    let args = vec![
        "-c".to_string(),
        "printf persistent-preview-%s \"$VAK_TEST_RUNTIME_HOME\"".to_string(),
    ];
    let environment = vec![("VAK_TEST_RUNTIME_HOME".to_string(), "isolated".to_string())];
    let mut child = vak_tools::broker::spawn_persistent_worker(
        std::path::Path::new(env!("CARGO_BIN_EXE_vak-tool-worker")),
        root.path(),
        "sh",
        &args,
        &environment,
        None,
    )
    .await
    .expect("persistent worker starts");
    let mut stdout = child.stdout.take().expect("worker stdout");
    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .await
        .expect("read output");
    let status = child.wait().await.expect("worker exits");

    assert!(status.success());
    assert_eq!(output, "persistent-preview-isolated");
}
