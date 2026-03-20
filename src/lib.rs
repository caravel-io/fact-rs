pub mod components;

pub mod filesystem;

use crate::components::{cpu, kernel, memory, mount, network, os, uptime};
use anyhow::Result;
use rayon::prelude::*;
use serde_json::{Map, Value};

pub trait Collector: Send + Sync {
    fn name(&self) -> &'static str;
    fn collect(&self) -> Result<serde_json::Value>;
}

pub fn run() -> Result<()> {
    // Register all the components here. Each component
    // implements the Component trait
    let components: Vec<Box<dyn Collector>> = vec![
        Box::new(kernel::KernelComponent::new()),
        Box::new(cpu::CPUComponent::new()),
        Box::new(memory::MemoryComponent::new()),
        Box::new(os::OSComponent::new()),
        Box::new(uptime::UptimeComponent::new()),
        Box::new(network::NetworkComponent::new()),
        Box::new(mount::MountComponent::new()),
    ];

    // Build all the components in parallel into pairs of information
    let pairs: Vec<(String, Value)> = components
        .par_iter()
        .filter_map(|c| {
            let name = c.name().to_string();
            match c.collect() {
                Ok(v) => Some((name, v)),
                Err(e) => {
                    tracing::warn!(component = %name, error = ?e, "collector failed");
                    None
                }
            }
        })
        .collect();

    // Collect all the pairs into the main facts structure
    let facts: Map<String, Value> = pairs.into_iter().collect();

    let _j = serde_json::to_string(&Value::Object(facts))?;
    println!("{}", _j);
    Ok(())
}
