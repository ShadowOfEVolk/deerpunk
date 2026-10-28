//! Host console → command buffer (implemented by host `CmdSystem`).

pub trait ConsoleCmdSink {
    fn cmd_names(&self) -> Vec<String>;
    fn cmd_add(&self, command: String);
    fn cmd_process(&self);
    fn cmd_autocomplete(&self, line: &str, name_pool: &[String]) -> Vec<String>;
}
