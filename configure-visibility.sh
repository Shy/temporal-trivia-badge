#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
settings="$project_dir/.env.temporal"
if [ ! -f "$settings" ] && [ -f "$project_dir/../../TrafficLight/.env" ]; then
    settings="$project_dir/../../TrafficLight/.env"
fi

if [ -f "$settings" ]; then
    set -a
    # shellcheck disable=SC1090
    . "$settings"
    set +a
fi

: "${TEMPORAL_ADDRESS:?missing TEMPORAL_ADDRESS}"
: "${TEMPORAL_NAMESPACE:?missing TEMPORAL_NAMESPACE}"
: "${TEMPORAL_API_KEY:?missing TEMPORAL_API_KEY}"

address=${TEMPORAL_ADDRESS#https://}
existing=$(temporal operator search-attribute list \
    --address "$address" \
    --namespace "$TEMPORAL_NAMESPACE" \
    --tls)

register() {
    name=$1
    type=$2
    if printf '%s\n' "$existing" | grep -q "$name"; then
        echo "$name already registered"
        return
    fi
    temporal operator search-attribute create \
        --address "$address" \
        --namespace "$TEMPORAL_NAMESPACE" \
        --tls \
        --name "$name" \
        --type "$type"
}

register TriviaGameId Keyword
register TriviaStatus Keyword
register TriviaWinners KeywordList
register TriviaBadgeCount Int
register TriviaCorrectAnswers Int
register TriviaWrongAnswers Int
register TriviaCrashes Int
register TriviaReassignments Int
