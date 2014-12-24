#!/bin/bash
set -e
DIR=$( cd `dirname "${BASH_SOURCE[0]}"` > /dev/null && pwd)

function die() {
    echo FATAL: $1
    exit 1
}

if [[ -z $1 ]]; then
    die "Must pass a host to install to as first argument"
fi

if [[ -z $2 ]]; then
    die "Must pass a secret as second argument"
fi

if [[ -z $3 ]]; then
    die "Must pass a playbook as third argument"
fi

HOST=$1
export DEPLOYER_SECRET=$2
export DEPLOYER_PLAYBOOK=$3

cp $DIR/../release/deployer.linux $DIR/../deploy/ansible/roles/install/files/

ansible-playbook -i $HOST, deploy/ansible/install.yml
