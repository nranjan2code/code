#!/bin/sh
# Removes the lab's two machines and everything they stored.
docker rm -f lab-desk lab-away
docker volume rm -f lab-desk-data lab-away-data lab-remote
