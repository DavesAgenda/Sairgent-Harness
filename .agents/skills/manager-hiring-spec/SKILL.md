---
name: Manager Hiring Spec
description: Structured manager skill for defining new agents, team-gap repairs, and org restructures.
---

# Manager Hiring Spec

Use this skill when a manager or `Sairgent Agent` needs to:
- identify a missing team capability
- propose a new hire
- repair a reporting line
- rewrite an agent definition so it is creation-ready

## Required Output

Return a single JSON object with these fields:

```json
{
  "roleIntent": "",
  "orgClass": "manager | lead_ic | specialist",
  "mission": "",
  "teamGoalAlignment": [],
  "domainOwnership": [],
  "requiredSkills": [],
  "requiredTools": [],
  "reportingLine": {
    "managerAgentId": "",
    "managerName": ""
  },
  "delegationPolicy": "must_delegate_when_fit_exists | may_delegate | may_not_delegate",
  "reviewPolicy": "synthesize_only | direct_allowed",
  "qualityCriteria": "",
  "hireOrRestructureJustification": "",
  "operatorReadySpec": {
    "name": "",
    "role": "",
    "mission": "",
    "provider": "",
    "model": ""
  }
}
```

## Rules

- Do not emit prose outside the JSON object.
- Do not use vague role definitions like "help with marketing" or "be strategic."
- Domain ownership must be explicit and bounded.
- Quality criteria must describe what good output looks like and how the manager should review it.
- If proposing a manager, define what the manager owns and what must be delegated.
- If proposing a specialist, keep `reviewPolicy` as `direct_allowed` unless there is a strong reason not to.
- If information is missing, infer the minimum needed to make the spec creation-ready and state that in `hireOrRestructureJustification`.

## Validation Checklist

Reject and rewrite the output if any of these are true:
- `mission` is generic or overlaps existing roles without a differentiation statement
- `domainOwnership` is empty
- `teamGoalAlignment` is empty
- `requiredSkills` and `requiredTools` are both empty
- `operatorReadySpec` is incomplete
- the reporting line is missing for a non-root role
