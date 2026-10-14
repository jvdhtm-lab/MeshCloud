//! HTTP client for the control-plane API.

use std::time::Duration;

use anyhow::Context;
use mesh_agent_protocol::{
    AgentHeartbeatRequest, AgentHeartbeatResponse, AgentView, RegisterAgentRequest,
    RegisterAgentResponse,
};
use mesh_protocol::ids::AgentId;
use mesh_protocol::{
    ClaimedJobResponse, CompleteJobRequest, CompleteJobResponse, DelegateJobRequest,
    DelegateJobResponse, FailJobRequest, FailJobResponse, HeartbeatRequest, HeartbeatResponse,
    JobId, NodeId, RegisterNodeRequest, RegisterNodeResponse, WaitJobResponse,
};

/// A thin HTTP client for the endpoints a worker uses.
pub struct ControlPlaneClient {
    http: reqwest::Client,
    base: String,
}

impl ControlPlaneClient {
    /// Build a client for the given control-plane base URL.
    pub fn new(base: impl Into<String>) -> anyhow::Result<Self> {
        let base = base.into().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { http, base })
    }

    /// Register this worker with the control plane.
    pub async fn register(
        &self,
        request: &RegisterNodeRequest,
    ) -> anyhow::Result<RegisterNodeResponse> {
        let response = self
            .http
            .post(format!("{}/api/v1/nodes/register", self.base))
            .json(request)
            .send()
            .await
            .context("register request failed")?
            .error_for_status()
            .context("control plane rejected registration")?
            .json()
            .await
            .context("invalid registration response")?;
        Ok(response)
    }

    /// Send a heartbeat with a fresh signed offer.
    pub async fn heartbeat(
        &self,
        node_id: &NodeId,
        request: &HeartbeatRequest,
    ) -> anyhow::Result<HeartbeatResponse> {
        let response = self
            .http
            .post(format!(
                "{}/api/v1/nodes/{}/heartbeat",
                self.base,
                node_id.as_str()
            ))
            .json(request)
            .send()
            .await
            .context("heartbeat request failed")?
            .error_for_status()
            .context("control plane rejected heartbeat")?
            .json()
            .await
            .context("invalid heartbeat response")?;
        Ok(response)
    }

    /// Claim the next eligible job, or `None` (204) when there is no work.
    pub async fn claim_job(
        &self,
        worker_id: &NodeId,
        workloads: &[String],
    ) -> anyhow::Result<Option<ClaimedJobResponse>> {
        let url = format!(
            "{}/api/v1/workers/{}/jobs/next?workloads={}",
            self.base,
            worker_id.as_str(),
            workloads.join(",")
        );
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("claim request failed")?;
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let claimed = response
            .error_for_status()
            .context("control plane rejected claim")?
            .json()
            .await
            .context("invalid claim response")?;
        Ok(Some(claimed))
    }

    /// Report a completed job attempt.
    pub async fn complete_job(
        &self,
        job_id: &JobId,
        request: &CompleteJobRequest,
    ) -> anyhow::Result<CompleteJobResponse> {
        let response = self
            .http
            .post(format!(
                "{}/api/v1/jobs/{}/complete",
                self.base,
                job_id.as_str()
            ))
            .json(request)
            .send()
            .await
            .context("complete request failed")?
            .error_for_status()
            .context("control plane rejected completion")?
            .json()
            .await
            .context("invalid complete response")?;
        Ok(response)
    }

    /// Report a failed job attempt.
    pub async fn fail_job(
        &self,
        job_id: &JobId,
        request: &FailJobRequest,
    ) -> anyhow::Result<FailJobResponse> {
        let response = self
            .http
            .post(format!(
                "{}/api/v1/jobs/{}/fail",
                self.base,
                job_id.as_str()
            ))
            .json(request)
            .send()
            .await
            .context("fail request failed")?
            .error_for_status()
            .context("control plane rejected failure report")?
            .json()
            .await
            .context("invalid fail response")?;
        Ok(response)
    }

    /// Register an agent hosted by this worker.
    pub async fn register_agent(
        &self,
        request: &RegisterAgentRequest,
    ) -> anyhow::Result<RegisterAgentResponse> {
        let response = self
            .http
            .post(format!("{}/api/v1/agents/register", self.base))
            .json(request)
            .send()
            .await
            .context("agent register request failed")?
            .error_for_status()
            .context("control plane rejected agent registration")?
            .json()
            .await
            .context("invalid agent register response")?;
        Ok(response)
    }

    /// Send an agent heartbeat.
    pub async fn heartbeat_agent(
        &self,
        agent_id: &AgentId,
        request: &AgentHeartbeatRequest,
    ) -> anyhow::Result<AgentHeartbeatResponse> {
        let response = self
            .http
            .post(format!(
                "{}/api/v1/agents/{}/heartbeat",
                self.base,
                agent_id.as_str()
            ))
            .json(request)
            .send()
            .await
            .context("agent heartbeat request failed")?
            .error_for_status()
            .context("control plane rejected agent heartbeat")?
            .json()
            .await
            .context("invalid agent heartbeat response")?;
        Ok(response)
    }

    /// Claim the next agent task assigned to an agent, or `None` (204).
    pub async fn claim_agent_job(
        &self,
        agent_id: &AgentId,
        workloads: &[String],
    ) -> anyhow::Result<Option<ClaimedJobResponse>> {
        let url = format!(
            "{}/api/v1/agents/{}/jobs/next?workloads={}",
            self.base,
            agent_id.as_str(),
            workloads.join(",")
        );
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("agent claim request failed")?;
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let claimed = response
            .error_for_status()
            .context("control plane rejected agent claim")?
            .json()
            .await
            .context("invalid agent claim response")?;
        Ok(Some(claimed))
    }

    /// Delegate a child job from a parent.
    pub async fn delegate_job(
        &self,
        parent: &JobId,
        request: &DelegateJobRequest,
    ) -> anyhow::Result<DelegateJobResponse> {
        let response = self
            .http
            .post(format!(
                "{}/api/v1/jobs/{}/delegate",
                self.base,
                parent.as_str()
            ))
            .json(request)
            .send()
            .await
            .context("delegate request failed")?
            .error_for_status()
            .context("control plane rejected delegation")?
            .json()
            .await
            .context("invalid delegate response")?;
        Ok(response)
    }

    /// Mark a job as waiting on its children.
    pub async fn wait_job(&self, job_id: &JobId) -> anyhow::Result<WaitJobResponse> {
        let response = self
            .http
            .post(format!(
                "{}/api/v1/jobs/{}/wait",
                self.base,
                job_id.as_str()
            ))
            .send()
            .await
            .context("wait request failed")?
            .error_for_status()
            .context("control plane rejected wait")?
            .json()
            .await
            .context("invalid wait response")?;
        Ok(response)
    }

    /// List registered agents.
    pub async fn fetch_agents(&self) -> anyhow::Result<Vec<AgentView>> {
        let agents = self
            .http
            .get(format!("{}/api/v1/agents", self.base))
            .send()
            .await
            .context("agents request failed")?
            .error_for_status()
            .context("control plane rejected agents request")?
            .json()
            .await
            .context("invalid agents response")?;
        Ok(agents)
    }
}
