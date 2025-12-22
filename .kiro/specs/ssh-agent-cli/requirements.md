# Requirements Document

## Introduction

This document specifies requirements for enhancing RustConn with SSH Agent management capabilities, export functionality for connection configurations, and a Command Line Interface (CLI) for automated testing and scripting. These features enable better SSH key management, interoperability with other tools, and automated testing workflows.

## Glossary

- **RustConn**: The connection manager application being enhanced
- **SSH Agent**: A program that holds private keys used for SSH public key authentication
- **SSH Key**: A cryptographic key pair used for SSH authentication
- **CLI**: Command Line Interface for non-GUI operations
- **Export**: Converting RustConn connections to external formats
- **Import**: Converting external formats to RustConn connections (existing feature)
- **Ansible Inventory**: YAML/INI file format used by Ansible for host definitions
- **SSH Config**: OpenSSH client configuration file format (~/.ssh/config)
- **Remmina**: Linux remote desktop client with its own connection file format
- **Asbru-CM**: Connection manager with YAML-based configuration

## Requirements

### Requirement 1: SSH Agent Management

**User Story:** As a system administrator, I want to manage SSH keys in the ssh-agent from within RustConn, so that I can easily add keys and select them for connections without manual terminal commands.

#### Acceptance Criteria

1. WHEN a user opens Settings THEN the System SHALL display an SSH Agent section with agent status and controls
2. WHEN a user clicks "Start SSH Agent" button THEN the System SHALL execute `eval $(ssh-agent -s)` and capture the agent socket path
3. WHEN a user clicks "Add Key" THEN the System SHALL display a file chooser for selecting SSH private key files
4. WHEN a user selects a key file THEN the System SHALL execute `ssh-add <key_path>` to add the key to the agent
5. WHEN a key requires a passphrase THEN the System SHALL prompt for the passphrase and pass it to ssh-add
6. WHEN displaying agent status THEN the System SHALL show list of currently loaded keys via `ssh-add -l`
7. WHEN a user removes a key THEN the System SHALL execute `ssh-add -d <key_path>` to remove it from the agent

### Requirement 2: SSH Key Selection in Connections

**User Story:** As a user, I want to select SSH keys from the agent when configuring SSH connections, so that I can easily use managed keys without specifying file paths.

#### Acceptance Criteria

1. WHEN a user edits an SSH connection THEN the System SHALL display a dropdown for key selection
2. WHEN populating the key dropdown THEN the System SHALL list keys from ssh-agent and keys from file paths
3. WHEN a user selects an agent key THEN the System SHALL store the key fingerprint for identification
4. WHEN connecting with an agent key THEN the System SHALL use the SSH agent for authentication
5. WHEN the selected agent key is not available THEN the System SHALL display an error and offer alternatives

### Requirement 3: Export to Ansible Inventory

**User Story:** As a DevOps engineer, I want to export my SSH connections to Ansible inventory format, so that I can use my connection definitions with Ansible automation.

#### Acceptance Criteria

1. WHEN a user selects "Export to Ansible" THEN the System SHALL generate a valid Ansible inventory file
2. WHEN exporting SSH connections THEN the System SHALL include ansible_host, ansible_user, and ansible_ssh_private_key_file
3. WHEN exporting connections with groups THEN the System SHALL organize hosts under group sections
4. WHEN a connection has custom port THEN the System SHALL include ansible_port variable
5. WHEN parsing exported inventory THEN Ansible SHALL accept the file without errors
6. WHEN serializing to Ansible format THEN the System SHALL preserve host and authentication data for round-trip consistency

### Requirement 4: Export to SSH Config

**User Story:** As a user, I want to export my connections to SSH config format, so that I can use them with standard SSH tools and share with colleagues.

#### Acceptance Criteria

1. WHEN a user selects "Export to SSH Config" THEN the System SHALL generate a valid OpenSSH config file
2. WHEN exporting SSH connections THEN the System SHALL include Host, HostName, User, Port, and IdentityFile directives
3. WHEN a connection has a custom name THEN the System SHALL use it as the Host alias
4. WHEN exporting THEN the System SHALL handle special characters in hostnames correctly
5. WHEN parsing exported config THEN OpenSSH SHALL accept the file without errors
6. WHEN serializing to SSH config format THEN the System SHALL preserve connection data for round-trip consistency

### Requirement 5: Export to Remmina

**User Story:** As a user, I want to export my connections to Remmina format, so that I can share configurations with colleagues who use Remmina.

#### Acceptance Criteria

1. WHEN a user selects "Export to Remmina" THEN the System SHALL generate valid Remmina .remmina files
2. WHEN exporting SSH connections THEN the System SHALL create SSH protocol files with server, username, and ssh_privatekey
3. WHEN exporting RDP connections THEN the System SHALL create RDP protocol files with server, username, and domain
4. WHEN exporting VNC connections THEN the System SHALL create VNC protocol files with server and port
5. WHEN importing the exported files THEN Remmina SHALL recognize and load them correctly
6. WHEN serializing to Remmina format THEN the System SHALL preserve protocol-specific data for round-trip consistency

### Requirement 6: Export to Asbru-CM

**User Story:** As a user migrating from Asbru-CM, I want to export my connections back to Asbru format, so that I can maintain compatibility with existing workflows.

#### Acceptance Criteria

1. WHEN a user selects "Export to Asbru" THEN the System SHALL generate a valid Asbru-CM YAML configuration
2. WHEN exporting connections THEN the System SHALL include all connection properties in Asbru format
3. WHEN exporting groups THEN the System SHALL preserve the folder hierarchy
4. WHEN importing the exported file THEN Asbru-CM SHALL recognize and load connections correctly
5. WHEN serializing to Asbru format THEN the System SHALL preserve connection data for round-trip consistency

### Requirement 7: CLI Interface

**User Story:** As a power user, I want a command-line interface for RustConn, so that I can automate connection management and integrate with scripts.

#### Acceptance Criteria

1. WHEN a user runs `rustconn-cli list` THEN the System SHALL display all connections in tabular format
2. WHEN a user runs `rustconn-cli connect <name>` THEN the System SHALL initiate a connection by name or ID
3. WHEN a user runs `rustconn-cli add` with parameters THEN the System SHALL create a new connection
4. WHEN a user runs `rustconn-cli export --format <format>` THEN the System SHALL export connections to the specified format
5. WHEN a user runs `rustconn-cli import --format <format> <file>` THEN the System SHALL import connections from the file
6. WHEN a user runs `rustconn-cli test-connection <name>` THEN the System SHALL verify connectivity and report status
7. WHEN CLI commands fail THEN the System SHALL return appropriate exit codes and error messages
8. WHEN a user runs `rustconn-cli --help` THEN the System SHALL display usage information for all commands

### Requirement 8: CLI Connection Testing

**User Story:** As a DevOps engineer, I want to test connections via CLI, so that I can verify connectivity in automated pipelines.

#### Acceptance Criteria

1. WHEN a user runs `rustconn-cli test <name>` THEN the System SHALL attempt to connect and report success/failure
2. WHEN testing SSH connections THEN the System SHALL verify SSH handshake and authentication
3. WHEN testing RDP connections THEN the System SHALL verify RDP port accessibility
4. WHEN testing VNC connections THEN the System SHALL verify VNC port accessibility and protocol handshake
5. WHEN a test fails THEN the System SHALL display detailed error information
6. WHEN running batch tests THEN the System SHALL support testing multiple connections and summarizing results

### Requirement 9: Import Verification

**User Story:** As a user, I want to verify that imports preserve all connection parameters, so that I can trust the import functionality.

#### Acceptance Criteria

1. WHEN importing SSH connections THEN the System SHALL preserve hostname, port, username, and key path
2. WHEN importing RDP connections THEN the System SHALL preserve hostname, port, username, and domain
3. WHEN importing connections with special characters THEN the System SHALL handle encoding correctly
4. WHEN import completes THEN the System SHALL display a summary of imported connections with any warnings
5. WHEN round-trip testing (export then import) THEN the System SHALL produce equivalent connection data

