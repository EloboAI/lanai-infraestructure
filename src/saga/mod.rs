use async_trait::async_trait;
use log::{info, error, warn};
use std::fmt::Debug;

#[async_trait]
pub trait SagaStep: Send + Sync + Debug {
    type Context;
    type Error: Debug + std::fmt::Display;

    async fn execute(&self, context: &mut Self::Context) -> Result<(), Self::Error>;
    async fn compensate(&self, context: &mut Self::Context);
}

pub struct SagaOrchestrator<C, E> {
    steps: Vec<Box<dyn SagaStep<Context = C, Error = E>>>,
}

impl<C, E> SagaOrchestrator<C, E> 
where 
    E: Debug + std::fmt::Display,
    C: Debug
{
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn add_step(&mut self, step: Box<dyn SagaStep<Context = C, Error = E>>) {
        self.steps.push(step);
    }

    pub async fn run(&self, mut context: C) -> Result<C, E> {
        info!("🎬 Starting Saga with context: {:?}", context);
        let mut executed_steps = Vec::new();

        for (i, step) in self.steps.iter().enumerate() {
            info!("⚙️ Executing step {}: {:?}", i + 1, step);
            match step.execute(&mut context).await {
                Ok(_) => {
                    executed_steps.push(step);
                }
                Err(e) => {
                    error!("❌ Step {} failed: {}. Starting compensation...", i + 1, e);
                    self.compensate(executed_steps, &mut context).await;
                    return Err(e);
                }
            }
        }

        info!("🎉 Saga completed successfully!");
        Ok(context)
    }

    async fn compensate(&self, executed_steps: Vec<&Box<dyn SagaStep<Context = C, Error = E>>>, context: &mut C) {
        for step in executed_steps.into_iter().rev() {
            warn!("🔄 Compensating step: {:?}", step);
            step.compensate(context).await;
        }
    }
}
