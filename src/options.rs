#[derive(Debug, Clone)]
pub(crate) struct Options {
    python_version: (u32, u32),
    platform: String,
    always_true: Vec<String>,
    always_false: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            python_version: (3, 12),
            platform: String::from("linux"),
            always_true: Vec::new(),
            always_false: Vec::new(),
        }
    }
}

impl Options {
    pub(crate) fn new(
        python_version: (u32, u32),
        platform: String,
        always_true: Vec<String>,
        always_false: Vec<String>,
    ) -> Self {
        Self {
            python_version,
            platform,
            always_true,
            always_false,
        }
    }

    pub(crate) fn python_version(&self) -> (u32, u32) {
        self.python_version
    }

    pub(crate) fn platform(&self) -> &str {
        &self.platform
    }

    pub(crate) fn always_true(&self) -> &[String] {
        &self.always_true
    }

    pub(crate) fn always_false(&self) -> &[String] {
        &self.always_false
    }
}
