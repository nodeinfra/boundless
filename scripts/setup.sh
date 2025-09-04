#!/bin/bash

# =============================================================================
# Boundless Prover Node Setup Script - Modified Version
# Description: Automated installation handling both system and user components
# =============================================================================

set -euo pipefail

# Color variables
CYAN='\033[0;36m'
LIGHTBLUE='\033[1;34m'
RED='\033[0;31m'
GREEN='\033[0;32m'
PURPLE='\033[0;35m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
RESET='\033[0m'

# Determine actual user and home directory
if [[ $EUID -eq 0 ]]; then
    ACTUAL_USER=${SUDO_USER:-$USER}
    ACTUAL_HOME=$(eval echo "~$ACTUAL_USER")
else
    ACTUAL_USER=$USER
    ACTUAL_HOME=$HOME
fi

# Constants
SCRIPT_NAME="$(basename "$0")"
LOG_FILE="$ACTUAL_HOME/log/boundless_prover_setup.log"
ERROR_LOG="$ACTUAL_HOME/log/boundless_prover_error.log"
INSTALL_DIR="$ACTUAL_HOME/boundless"
COMPOSE_FILE="$INSTALL_DIR/compose.yml"
BROKER_CONFIG="$INSTALL_DIR/broker.toml"

# Exit codes
EXIT_SUCCESS=0
EXIT_OS_CHECK_FAILED=1
EXIT_DPKG_ERROR=2
EXIT_DEPENDENCY_FAILED=3
EXIT_GPU_ERROR=4
EXIT_NETWORK_ERROR=5
EXIT_USER_ABORT=6
EXIT_UNKNOWN=99

# Flags
ALLOW_ROOT=false
FORCE_RECLONE=false
START_IMMEDIATELY=false

# Parse command-line arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --allow-root)
            ALLOW_ROOT=true
            shift
            ;;
        --force-reclone)
            FORCE_RECLONE=true
            shift
            ;;
        --start-immediately)
            START_IMMEDIATELY=true
            shift
            ;;
        --help)
            echo "Usage: $0 [options]"
            echo "Options:"
            echo "  --allow-root        Allow running as root without prompting"
            echo "  --force-reclone     Automatically delete and re-clone the directory if it exists"
            echo "  --start-immediately Automatically start the prover after installation"
            echo "  --help              Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Functions
info() {
    printf "${CYAN}[INFO]${RESET} %s\n" "$1"
    echo "[INFO] $(date '+%Y-%m-%d %H:%M:%S') - $1" >> "$LOG_FILE"
}

success() {
    printf "${GREEN}[SUCCESS]${RESET} %s\n" "$1"
    echo "[SUCCESS] $(date '+%Y-%m-%d %H:%M:%S') - $1" >> "$LOG_FILE"
}

error() {
    printf "${RED}[ERROR]${RESET} %s\n" "$1" >&2
    echo "[ERROR] $(date '+%Y-%m-%d %H:%M:%S') - $1" >> "$LOG_FILE"
    echo "[ERROR] $(date '+%Y-%m-%d %H:%M:%S') - $1" >> "$ERROR_LOG"
}

warning() {
    printf "${YELLOW}[WARNING]${RESET} %s\n" "$1"
    echo "[WARNING] $(date '+%Y-%m-%d %H:%M:%S') - $1" >> "$LOG_FILE"
}

# Function to run commands as actual user
run_as_user() {
    if [[ $EUID -eq 0 ]]; then
        sudo -u "$ACTUAL_USER" -H "$@"
    else
        "$@"
    fi
}

# Function to run commands as root (system packages)
run_as_root() {
    if [[ $EUID -eq 0 ]]; then
        "$@"
    else
        if command -v sudo &> /dev/null; then
            sudo "$@"
        else
            error "This script requires root privileges for system package installation"
            error "Please install sudo or run as root"
            exit 1
        fi
    fi
}

# Check OS compatibility
check_os() {
    info "Checking operating system compatibility..."
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        if [[ "${ID,,}" != "ubuntu" ]]; then
            error "Unsupported OS: $NAME. This script is for Ubuntu."
            exit $EXIT_OS_CHECK_FAILED
        elif [[ "${VERSION_ID,,}" != "22.04" && "${VERSION_ID,,}" != "20.04" ]]; then
            warning "Tested on Ubuntu 20.04/22.04. Your version: $VERSION_ID"
            read -e -p "Continue anyway? (y/N): " response
            if [[ ! "$response" =~ ^[yY]$ ]]; then
                exit $EXIT_USER_ABORT
            fi
        else
            info "Operating System: $PRETTY_NAME"
        fi
    else
        error "/etc/os-release not found. Unable to determine OS."
        exit $EXIT_OS_CHECK_FAILED
    fi
}

# Update system (requires root)
update_system() {
    info "Updating system packages..."
    run_as_root apt update -y
    run_as_root apt upgrade -y
    success "System packages updated"
}

# Install basic dependencies (requires root)
install_basic_deps() {
    local packages=(
        curl iptables build-essential git wget lz4 jq make gcc nano
        automake autoconf tmux htop nvme-cli libgbm1 pkg-config
        libssl-dev tar clang bsdmainutils ncdu unzip libleveldb-dev
        libclang-dev ninja-build nvtop ubuntu-drivers-common
        gnupg ca-certificates lsb-release postgresql-client
    )
    info "Installing basic dependencies..."
    run_as_root apt install -y "${packages[@]}"
    success "Basic dependencies installed"
}

# Install GPU drivers (requires root)
install_gpu_drivers() {
    info "Installing NVIDIA drivers version 575-open..."
    distribution=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')
    run_as_root bash -c "curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey | gpg --dearmor -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg"
    run_as_root bash -c "curl -s -L https://nvidia.github.io/libnvidia-container/$distribution/libnvidia-container.list | tee /etc/apt/sources.list.d/nvidia-container-toolkit.list"
    run_as_root apt update -y
    run_as_root apt install -y nvidia-driver-575-open
    success "NVIDIA drivers 575-open installed"
}

# Install Docker (requires root)
install_docker() {
    if command -v docker &> /dev/null; then
        info "Docker already installed"
    else
        info "Installing Docker..."
        run_as_root apt install -y apt-transport-https ca-certificates curl gnupg-agent software-properties-common
        run_as_root bash -c "curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg"
        run_as_root bash -c 'echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null'
        run_as_root apt update -y
        run_as_root apt install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
        run_as_root systemctl enable docker
        run_as_root systemctl start docker
        success "Docker installed"
    fi
    # Always try to add user to docker group
    run_as_root usermod -aG docker "$ACTUAL_USER"
    success "User $ACTUAL_USER added to docker group"
}

# Install NVIDIA Container Toolkit (requires root)
install_nvidia_toolkit() {
    info "Installing NVIDIA Container Toolkit..."
    distribution=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')
    run_as_root bash -c "curl -s -L https://nvidia.github.io/nvidia-docker/gpgkey | apt-key add -"
    run_as_root bash -c "curl -s -L https://nvidia.github.io/nvidia-docker/$distribution/nvidia-docker.list | tee /etc/apt/sources.list.d/nvidia-docker.list"
    run_as_root apt update -y
    run_as_root apt install -y nvidia-docker2
    run_as_root mkdir -p /etc/docker
    run_as_root tee /etc/docker/daemon.json > /dev/null <<'EOF'
{
    "default-runtime": "nvidia",
    "runtimes": {
        "nvidia": {
            "path": "nvidia-container-runtime",
            "runtimeArgs": []
        }
    }
}
EOF
    run_as_root systemctl restart docker
    success "NVIDIA Container Toolkit installed"
}

# Install CUDA Toolkit (requires root)
install_cuda() {
    info "Installing CUDA Toolkit 12.9..."
    distribution=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"'| tr -d '\.')
    cd /tmp
    run_as_root wget "https://developer.download.nvidia.com/compute/cuda/repos/$distribution/$(uname -m)/cuda-keyring_1.1-1_all.deb"
    run_as_root dpkg -i cuda-keyring_1.1-1_all.deb
    run_as_root rm -f cuda-keyring_1.1-1_all.deb
    run_as_root apt-get update
    run_as_root apt-get install -y cuda-toolkit-12-9
    success "CUDA Toolkit 12.9 installed"
}

# Install Rust (as user)
install_rust() {
    if run_as_user bash -c 'command -v rustc &> /dev/null'; then
        info "Rust already installed for user $ACTUAL_USER"
        return
    fi
    info "Installing Rust for user $ACTUAL_USER..."
    run_as_user bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'
    run_as_user bash -c 'source "$HOME/.cargo/env" && rustup update'
    success "Rust installed for user $ACTUAL_USER"
}

# Install Just (as user)
install_just() {
    if run_as_user bash -c 'command -v just &> /dev/null'; then
        info "Just already installed for user $ACTUAL_USER"
        return
    fi
    info "Installing Just command runner for user $ACTUAL_USER..."
    run_as_user mkdir -p "$ACTUAL_HOME/.local/bin"
    run_as_user bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to "$HOME/.local/bin"'
    # Add to PATH if not already there
    run_as_user bash -c 'if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then echo "export PATH=\"\$HOME/.local/bin:\$PATH\"" >> ~/.bashrc; fi'
    success "Just installed for user $ACTUAL_USER"
}

# Install Rust dependencies (as user)
install_rust_deps() {
    info "Installing Rust dependencies for user $ACTUAL_USER..."

    # Install rzup and the RISC Zero Rust toolchain
    info "Installing rzup for user $ACTUAL_USER..."
    run_as_user bash -c 'curl -L https://risczero.com/install | bash'
    
    # Install RISC Zero Rust toolchain
    run_as_user bash -c 'export PATH="$PATH:$HOME/.risc0/bin" && "$HOME/.risc0/bin/rzup" install rust'

    # Detect the RISC Zero toolchain
    TOOLCHAIN=$(run_as_user bash -c 'source "$HOME/.cargo/env" && rustup toolchain list | grep risc0 | head -1 | cut -d" " -f1')
    if [ -z "$TOOLCHAIN" ]; then
        error "No RISC Zero toolchain found after installation"
        exit $EXIT_DEPENDENCY_FAILED
    fi
    info "Using RISC Zero toolchain: $TOOLCHAIN"

    # Install cargo-risczero
    info "Installing cargo-risczero for user $ACTUAL_USER..."
    run_as_user bash -c 'source "$HOME/.cargo/env" && cargo install cargo-risczero'
    run_as_user bash -c 'export PATH="$PATH:$HOME/.risc0/bin" && "$HOME/.risc0/bin/rzup" install cargo-risczero'

    # Install bento-client with the RISC Zero toolchain
    info "Installing bento-client for user $ACTUAL_USER..."
    run_as_user bash -c "source \"\$HOME/.cargo/env\" && RUSTUP_TOOLCHAIN=$TOOLCHAIN cargo install --locked --git https://github.com/risc0/risc0 bento-client --branch release-2.3 --bin bento_cli"

    # Install boundless-cli
    info "Installing boundless-cli for user $ACTUAL_USER..."
    run_as_user bash -c 'source "$HOME/.cargo/env" && cargo install --locked boundless-cli'

    # Update PATH for cargo binaries and CUDA
    run_as_user bash -c 'if [[ ":$PATH:" != *":$HOME/.cargo/bin:"* ]]; then echo "export PATH=\"\$HOME/.cargo/bin:\$PATH\"" >> ~/.bashrc; fi'
    run_as_user bash -c 'if [[ ":$PATH:" != *":/usr/local/cuda-12.9/bin:"* ]]; then echo "export PATH=/usr/local/cuda-12.9/bin:\$PATH" >> ~/.bashrc; echo "export LD_LIBRARY_PATH=/usr/local/cuda-12.9/lib64:\$LD_LIBRARY_PATH" >> ~/.bashrc; fi'

    success "Rust dependencies installed for user $ACTUAL_USER"
}

# Main installation flow
main() {
    echo -e "${BOLD}${CYAN}Boundless Prover Node Setup${RESET}"
    echo "========================================"
    
    # Create log directories as actual user
    run_as_user mkdir -p "$(dirname "$LOG_FILE")"
    run_as_user touch "$LOG_FILE"
    run_as_user touch "$ERROR_LOG"
    
    echo "[START] Installation started at $(date)" >> "$LOG_FILE"
    echo "[START] Installation started at $(date)" >> "$ERROR_LOG"
    
    info "Running as: $(whoami)"
    info "Installing for user: $ACTUAL_USER"
    info "Home directory: $ACTUAL_HOME"
    info "Logs will be saved to:"
    info "  - Full log: $LOG_FILE"
    info "  - Error log: $ERROR_LOG"
    echo

    if [[ $EUID -ne 0 ]]; then
        warning "Some operations require root privileges and will use sudo"
        info "Make sure you have sudo access"
    fi

    check_os
    update_system
    info "Installing system dependencies..."
    install_basic_deps
    install_gpu_drivers
    install_docker
    install_nvidia_toolkit
    install_cuda
    
    info "Installing user dependencies..."
    install_rust
    install_just
    install_rust_deps
    
    echo -e "\n${GREEN}${BOLD}Installation Complete!${RESET}"
    echo -e "${YELLOW}Please run 'source ~/.bashrc' or restart your terminal to update PATH${RESET}"
    echo -e "${YELLOW}You may need to log out and back in for Docker group membership to take effect${RESET}"
    echo "[SUCCESS] Installation completed successfully at $(date)" >> "$LOG_FILE"
}

# Run main
main "$@"