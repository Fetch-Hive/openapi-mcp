#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Ok = 0,
    Usage = 1,
    Policy = 2,
    SupplyChain = 3,
    Upstream = 4,
    Interrupted = 130,
}
