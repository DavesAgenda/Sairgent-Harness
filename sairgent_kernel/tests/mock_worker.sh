#!/bin/bash
read -r API_KEY
MODE=$1
AGENT_ROLE=$AGENT_ROLE

if [ "$MODE" == "execute_triage" ]; then
    if [ "$AGENT_ROLE" == "COO" ]; then
        # The manager delegates
        # The agent ID mapping is dynamic, but we can extract it from AGENT_SUBORDINATES
        SUB_1_ID=$(echo $AGENT_SUBORDINATES | grep -o -E '"id":"[^"]+"' | head -n 1 | cut -d '"' -f 4)
        SUB_2_ID=$(echo $AGENT_SUBORDINATES | grep -o -E '"id":"[^"]+"' | tail -n 1 | cut -d '"' -f 4)
        
        echo "{\"triage\": {\"action\": \"DELEGATE\", \"reasoning\": \"Delegating to subs\", \"delegation_swos\": {\"$SUB_1_ID\": \"Fix it\", \"$SUB_2_ID\": \"Pay for it\"}}}"
    else
        # Subordinates answer directly
        echo "{\"triage\": {\"action\": \"ANSWER_DIRECTLY\", \"reasoning\": \"I am a sub\", \"direct_answer\": \"Subordinate result from $AGENT_ROLE\"}}"
    fi
elif [ "$MODE" == "execute_synthesis" ]; then
    echo "{\"synthesis\": {\"action\": \"APPROVE_AND_REPLY\", \"reasoning\": \"All good\", \"final_response\": \"Final synthesized answer\"}}"
fi
