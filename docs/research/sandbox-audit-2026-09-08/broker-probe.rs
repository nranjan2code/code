use vak_tools::{ToolContext, SandboxEventSink};
fn main() {
 let rt=tokio::runtime::Runtime::new().unwrap();
 if std::env::args().nth(1).as_deref()==Some("__tool_worker") {std::process::exit(rt.block_on(vak_tools::broker::worker_main()));}
 rt.block_on(async {
 let cwd=std::env::current_exe().unwrap().parent().unwrap().join("fixture");
 std::fs::create_dir_all(&cwd).unwrap();
 let (sink,mut rx)=SandboxEventSink::new();
 let mut ctx=ToolContext::new(cwd);
 ctx.sandbox_sink=Some(sink);
 let tools=vak_tools::brokered_default_tools(std::env::current_exe().unwrap());
 let bash=tools.iter().find(|t|t.name()=="bash").unwrap();
 let out=bash.execute(&serde_json::json!({"command":"printf audit-broker-output"}),&ctx).await;
 assert!(!out.is_error,"{}",out.content);
 assert!(out.content.contains("audit-broker-output"));
 assert!(rx.try_recv().is_err());
 println!("Confirmed real broker: successful Bash result, zero sandbox events received.");
 let start=std::time::Instant::now();
 let timed=bash.execute(&serde_json::json!({"command":"printf audit-partial; sleep 3", "timeout_ms":1000}),&ctx).await;
 println!("Broker timeout: elapsed_ms={}, result={:?}",start.elapsed().as_millis(),timed.content);
 assert!(start.elapsed().as_millis() >= 2500);
 assert!(timed.is_error);
 assert!(!timed.content.contains("audit-partial"));
 });
}