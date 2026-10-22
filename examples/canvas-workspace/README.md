# Installable Canvas workspace example

This package is inert until installed and explicitly enabled through the normal
Plugins controls. Choose **More options → Canvas workspaces** in a conversation,
then start **Component parameters**. Saved work belongs to that conversation;
opening another conversation does not share its state or confer access.

The example demonstrates typed, revision-checked commands, undo/redo and selected
Agent context. It is not an engineering solver or a finished scientific plugin.
The schema contains domain names, labels, units and validation. The common host
does not hardcode them.

Package JavaScript is never imported into the application. Workspaces declare
an owned UI contract. Optional custom calculations are pure WebAssembly modules
run in disposable workers; see the developer guide in the Canvas design. Longer
or externally connected work uses the existing admitted execution tools.
