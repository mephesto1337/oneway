#[derive(Debug)]
pub struct Config {
    pub remission_count: usize,
    pub mtu: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            remission_count: 3,
            mtu: 1024,
        }
    }
}
