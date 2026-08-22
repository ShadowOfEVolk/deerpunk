pub trait CmdApi {
    fn execute(&self, command: &str);
}
