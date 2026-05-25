#!/bin/bash
read -r API_KEY
MODE=$1
AGENT_ROLE=$AGENT_ROLE

if [ "$MODE" == "execute_triage" ]; then
    if [ "$AGENT_ROLE" == "COO" ]; then
        # The manager delegates
        SUB_1_ID=$(echo $AGENT_SUBORDINATES | grep -o -E '"id":"[^"]+"' | head -n 1 | cut -d '"' -f 4)
        echo "{\"triage\": {\"action\": \"DELEGATE\", \"reasoning\": \"Delegating to subs\", \"delegation_swos\": {\"$SUB_1_ID\": \"Do the task.\"}}}"
    else
        # The subordinate emits an innovation report via stderr side-channel
        echo "{\"__sairgent_sidechannel\": \"innovation_report\", \"token\": \"$SAIRGENT_SIDECHANNEL_TOKEN\", \"report\": {\"title\": \"Repetitive Task Discovered\", \"context\": \"Doing the task.\", \"proposed_solution\": \"Automate this.\", \"estimated_impact\": \"High\"}}" >&2
        # And then completes normally
        echo "{\"triage\": {\"action\": \"ANSWER_DIRECTLY\", \"reasoning\": \"I am a sub\", \"direct_answer\": \"Subordinate result from $AGENT_ROLE\"}}"
    fi
elif [ "$MODE" == "execute_synthesis" ]; then
    echo "{\"synthesis\": {\"action\": \"APPROVE_AND_REPLY\", \"reasoning\": \"All good\", \"final_response\": \"Final synthesized answer\"}}"
fi
