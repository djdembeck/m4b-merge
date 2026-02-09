pub struct Config {
    pub verbose: bool,
}

impl Config {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}