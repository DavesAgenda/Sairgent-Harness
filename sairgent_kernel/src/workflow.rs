use crate::error::{KernelError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStepMode {
    Sequential,
    FanOut,
    Collect,
    Conditional,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowAssignee {
    CurrentAgent,
    RequestedAssignee,
    DirectReports,
    AgentIds(Vec<String>),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowCondition {
    pub variable: String,
    pub equals: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowStepTemplate {
    pub id: String,
    pub name: String,
    pub mode: WorkflowStepMode,
    pub assignee: WorkflowAssignee,
    pub prompt: String,
    pub when: Option<WorkflowCondition>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub entry_agent_id: String,
    pub steps: Vec<WorkflowStepTemplate>,
    pub review_required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkflowCompileContext {
    pub requested_assignee_agent_id: Option<String>,
    pub direct_report_ids: Vec<String>,
    pub variables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowStepRun {
    pub step_id: String,
    pub name: String,
    pub mode: WorkflowStepMode,
    pub assigned_agent_ids: Vec<String>,
    pub payloads: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowRun {
    pub template_id: String,
    pub template_name: String,
    pub entry_agent_id: String,
    pub review_required: bool,
    pub steps: Vec<WorkflowStepRun>,
}

fn render_prompt(template: &str, variables: &BTreeMap<String, String>) -> String {
    let mut output = template.to_string();
    for (key, value) in variables {
        output = output.replace(&format!("{{{{{}}}}}", key), value);
    }
    output
}

fn resolve_assignees(
    entry_agent_id: &str,
    assignee: &WorkflowAssignee,
    context: &WorkflowCompileContext,
) -> Result<Vec<String>> {
    let ids = match assignee {
        WorkflowAssignee::CurrentAgent => vec![entry_agent_id.to_string()],
        WorkflowAssignee::RequestedAssignee => context
            .requested_assignee_agent_id
            .clone()
            .map(|agent_id| vec![agent_id])
            .unwrap_or_else(|| vec![entry_agent_id.to_string()]),
        WorkflowAssignee::DirectReports => context.direct_report_ids.clone(),
        WorkflowAssignee::AgentIds(ids) => ids.clone(),
    };

    if ids.is_empty() {
        return Err(KernelError::Internal(
            "Workflow step resolved to zero assignees".to_string(),
        ));
    }

    Ok(ids)
}

pub fn compile_workflow(
    template: &WorkflowTemplate,
    context: &WorkflowCompileContext,
) -> Result<WorkflowRun> {
    let mut steps = Vec::new();

    for step in &template.steps {
        if let Some(condition) = &step.when {
            let Some(value) = context.variables.get(&condition.variable) else {
                continue;
            };
            if value != &condition.equals {
                continue;
            }
        }

        let assignees = resolve_assignees(&template.entry_agent_id, &step.assignee, context)?;
        let mut payloads = Vec::new();
        let rendered_prompt = render_prompt(&step.prompt, &context.variables);

        match step.mode {
            WorkflowStepMode::Sequential
            | WorkflowStepMode::Collect
            | WorkflowStepMode::Conditional => {
                for assignee in &assignees {
                    payloads.push(format!(
                        "WORKFLOW STEP\nStep: {}\nMode: {:?}\nAssigned agent: {}\nTask:\n{}",
                        step.name, step.mode, assignee, rendered_prompt
                    ));
                }
            }
            WorkflowStepMode::FanOut => {
                for assignee in &assignees {
                    payloads.push(format!(
                        "WORKFLOW STEP\nStep: {}\nMode: fan_out\nAssigned agent: {}\nTask:\n{}",
                        step.name, assignee, rendered_prompt
                    ));
                }
            }
        }

        steps.push(WorkflowStepRun {
            step_id: step.id.clone(),
            name: step.name.clone(),
            mode: step.mode.clone(),
            assigned_agent_ids: assignees,
            payloads,
        });
    }

    Ok(WorkflowRun {
        template_id: template.id.clone(),
        template_name: template.name.clone(),
        entry_agent_id: template.entry_agent_id.clone(),
        review_required: template.review_required,
        steps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_fan_out_and_collect_workflow() {
        let template = WorkflowTemplate {
            id: "wf-1".to_string(),
            name: "Research And Review".to_string(),
            entry_agent_id: "perry".to_string(),
            review_required: true,
            steps: vec![
                WorkflowStepTemplate {
                    id: "fanout".to_string(),
                    name: "Fan Out".to_string(),
                    mode: WorkflowStepMode::FanOut,
                    assignee: WorkflowAssignee::DirectReports,
                    prompt: "Research {{topic}} deeply.".to_string(),
                    when: None,
                },
                WorkflowStepTemplate {
                    id: "collect".to_string(),
                    name: "Collect".to_string(),
                    mode: WorkflowStepMode::Collect,
                    assignee: WorkflowAssignee::CurrentAgent,
                    prompt: "Review all submitted research on {{topic}}.".to_string(),
                    when: None,
                },
            ],
        };

        let run = compile_workflow(
            &template,
            &WorkflowCompileContext {
                requested_assignee_agent_id: None,
                direct_report_ids: vec!["felicity".to_string(), "lois".to_string()],
                variables: BTreeMap::from([("topic".to_string(), "pricing".to_string())]),
            },
        )
        .unwrap();

        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[0].assigned_agent_ids.len(), 2);
        assert!(run.steps[0].payloads[0].contains("pricing"));
        assert_eq!(run.steps[1].assigned_agent_ids, vec!["perry".to_string()]);
    }

    #[test]
    fn conditional_step_is_skipped_when_context_does_not_match() {
        let template = WorkflowTemplate {
            id: "wf-2".to_string(),
            name: "Conditional".to_string(),
            entry_agent_id: "perry".to_string(),
            review_required: false,
            steps: vec![WorkflowStepTemplate {
                id: "only-enterprise".to_string(),
                name: "Enterprise Followup".to_string(),
                mode: WorkflowStepMode::Conditional,
                assignee: WorkflowAssignee::CurrentAgent,
                prompt: "Handle enterprise-specific requirements.".to_string(),
                when: Some(WorkflowCondition {
                    variable: "segment".to_string(),
                    equals: "enterprise".to_string(),
                }),
            }],
        };

        let run = compile_workflow(
            &template,
            &WorkflowCompileContext {
                requested_assignee_agent_id: None,
                direct_report_ids: Vec::new(),
                variables: BTreeMap::from([("segment".to_string(), "startup".to_string())]),
            },
        )
        .unwrap();

        assert!(run.steps.is_empty());
    }
}
