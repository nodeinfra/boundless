#!/bin/bash

# =============================================================================
# Boundless Prover Node Setup Script
# Description: Automated installation and configuration of Boundless prover node
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

# Constants
SCRIPT_NAME="$(basename "$0")"
LOG_FILE="/home/ubuntu/log/boundless_prover_setup.log"
ERROR_LOG="/home/ubuntu/log/boundless_prover_error.log"
INSTALL_DIR="$HOME/boundless"
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

# Trap function for exit logging
cleanup_on_exit() {
    local exit_code=$?
    if [ $exit_code -ne 0 ]; then
        error "Installation failed with exit code: $exit_code"
        echo "[EXIT] Script exited with code: $exit_code at $(date)" >> "$ERROR_LOG"
        echo "[EXIT] Last command: ${BASH_COMMAND}" >> "$ERROR_LOG"
        echo "[EXIT] Line number: ${BASH_LINENO[0]}" >> "$ERROR_LOG"
        echo "[EXIT] Function stack: ${FUNCNAME[@]}" >> "$ERROR_LOG"

        echo -e "\n${RED}${BOLD}Installation Failed!${RESET}"
        echo -e "${YELLOW}Check error log at: $ERROR_LOG${RESET}"
        echo -e "${YELLOW}Check full log at: $LOG_FILE${RESET}"

        case $exit_code in
            $EXIT_DPKG_ERROR)
                echo -e "\n${RED}DPKG Configuration Error Detected!${RESET}"
                echo -e "${YELLOW}Please run the following command manually:${RESET}"
                echo -e "${BOLD}dpkg --configure -a${RESET}"
                echo -e "${YELLOW}Then re-run this installation script.${RESET}"
                ;;
            $EXIT_OS_CHECK_FAILED)
                echo -e "\n${RED}Operating system check failed!${RESET}"
                ;;
            $EXIT_DEPENDENCY_FAILED)
                echo -e "\n${RED}Dependency installation failed!${RESET}"
                ;;
            $EXIT_GPU_ERROR)
                echo -e "\n${RED}GPU configuration error!${RESET}"
                ;;
            $EXIT_NETWORK_ERROR)
                echo -e "\n${RED}Network configuration error!${RESET}"
                ;;
            $EXIT_USER_ABORT)
                echo -e "\n${YELLOW}Installation aborted by user.${RESET}"
                ;;
            *)
                echo -e "\n${RED}Unknown error occurred!${RESET}"
                ;;
        esac
    fi
}

# Set trap
trap cleanup_on_exit EXIT
trap 'echo "[SIGNAL] Caught signal ${?} at line ${LINENO}" >> "$ERROR_LOG"' ERR

# Network configurations
declare -A NETWORKS
NETWORKS["base"]="Base Mainnet|0x0b144e07a0826182b6b59788c34b32bfa86fb711|0x26759dbB201aFbA361Bec78E097Aa3942B0b4AB8|0x8C5a8b5cC272Fe2b74D18843CF9C3aCBc952a760|https://base-mainnet.beboundless.xyz"
NETWORKS["base-sepolia"]="Base Sepolia|0x0b144e07a0826182b6b59788c34b32bfa86fb711|0x6B7ABa661041164b8dB98E30AE1454d2e9D5f14b|0x8C5a8b5cC272Fe2b74D18843CF9C3aCBc952a760|https://base-sepolia.beboundless.xyz"
NETWORKS["eth-sepolia"]="Ethereum Sepolia|0x925d8331ddc0a1F0d96E68CF073DFE1d92b69187|0x13337C76fE2d1750246B68781ecEe164643b98Ec|0x7aAB646f23D1392d4522CFaB0b7FB5eaf6821d64|https://eth-sepolia.beboundless.xyz/"

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

prompt() {
    printf "${PURPLE}[INPUT]${RESET} %s" "$1"
}

# Check for dpkg errors
check_dpkg_status() {
    if dpkg --audit 2>&1 | grep -q "dpkg was interrupted"; then
        error "dpkg was interrupted - manual intervention required"
        return 1
    fi
    return 0
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

# Check if command exists
command_exists() {
    command -v "$1" &> /dev/null
}

# Check if package is installed
is_package_installed() {
    dpkg -s "$1" &> /dev/null
}

# Update system
update_system() {
    info "Updating system packages..."
    if ! check_dpkg_status; then
        exit $EXIT_DPKG_ERROR
    fi
    {
        if ! apt update -y 2>&1; then
            error "apt update failed"
            if apt update 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        fi
        if ! apt upgrade -y 2>&1; then
            error "apt upgrade failed"
            if apt upgrade 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        fi
    } >> "$LOG_FILE" 2>&1
    success "System packages updated"
}

# Install basic dependencies
install_basic_deps() {
    local packages=(
        curl iptables build-essential git wget lz4 jq make gcc nano
        automake autoconf tmux htop nvme-cli libgbm1 pkg-config
        libssl-dev tar clang bsdmainutils ncdu unzip libleveldb-dev
        libclang-dev ninja-build nvtop ubuntu-drivers-common
        gnupg ca-certificates lsb-release postgresql-client
    )
    info "Installing basic dependencies..."
    if ! check_dpkg_status; then
        exit $EXIT_DPKG_ERROR
    fi
    {
        if ! apt install -y "${packages[@]}" 2>&1; then
            error "Failed to install basic dependencies"
            if apt install -y "${packages[@]}" 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        fi
    } >> "$LOG_FILE" 2>&1
    success "Basic dependencies installed"
}

# Install GPU drivers
install_gpu_drivers() {
    info "Installing NVIDIA drivers version 575-open..."
    if ! check_dpkg_status; then
        exit $EXIT_DPKG_ERROR
    fi
    {
        # Add NVIDIA repository
        distribution=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')
        curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey | gpg --dearmor -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg
        curl -s -L https://nvidia.github.io/libnvidia-container/$distribution/libnvidia-container.list | tee /etc/apt/sources.list.d/nvidia-container-toolkit.list
        
        # Update package list
        if ! apt update -y 2>&1; then
            error "Failed to update package list for NVIDIA drivers"
            exit $EXIT_DEPENDENCY_FAILED
        fi
        
        # Install specific NVIDIA driver version
        if ! apt install -y nvidia-driver-575-open 2>&1; then
            error "Failed to install NVIDIA driver 575-open"
            if apt install -y nvidia-driver-575-open 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_GPU_ERROR
        fi
    } >> "$LOG_FILE" 2>&1
    success "NVIDIA drivers 575-open installed"
}

# Install Docker
install_docker() {
    if command_exists docker; then
        info "Docker already installed"
        return
    fi
    info "Installing Docker..."
    if ! check_dpkg_status; then
        exit $EXIT_DPKG_ERROR
    fi
    {
        if ! apt install -y apt-transport-https ca-certificates curl gnupg-agent software-properties-common 2>&1; then
            error "Failed to install Docker prerequisites"
            if apt install -y apt-transport-https 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        fi
        curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg
        echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null
        if ! apt update -y 2>&1; then
            error "Failed to update package list for Docker"
            exit $EXIT_DEPENDENCY_FAILED
        fi
        if ! apt install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin 2>&1; then
            error "Failed to install Docker"
            if apt install -y docker-ce 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        fi
        systemctl enable docker
        systemctl start docker
        usermod -aG docker $(logname 2>/dev/null || echo "$USER")
    } >> "$LOG_FILE" 2>&1
    success "Docker installed"
}

# Install NVIDIA Container Toolkit
install_nvidia_toolkit() {
    if is_package_installed "nvidia-docker2"; then
        info "NVIDIA Container Toolkit already installed"
        return
    fi
    info "Installing NVIDIA Container Toolkit..."
    if ! check_dpkg_status; then
        exit $EXIT_DPKG_ERROR
    fi
    {
        distribution=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')
        curl -s -L https://nvidia.github.io/nvidia-docker/gpgkey | apt-key add -
        curl -s -L https://nvidia.github.io/nvidia-docker/"$distribution"/nvidia-docker.list | tee /etc/apt/sources.list.d/nvidia-docker.list
        if ! apt update -y 2>&1; then
            error "Failed to update package list for NVIDIA toolkit"
            exit $EXIT_DEPENDENCY_FAILED
        fi
        if ! apt install -y nvidia-docker2 2>&1; then
            error "Failed to install NVIDIA Docker support"
            if apt install -y nvidia-docker2 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        fi
        mkdir -p /etc/docker
        tee /etc/docker/daemon.json <<EOF
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
        systemctl restart docker
    } >> "$LOG_FILE" 2>&1
    success "NVIDIA Container Toolkit installed"
}

# Install Rust
install_rust() {
    if command_exists rustc; then
        info "Rust already installed"
        return
    fi
    info "Installing Rust..."
    {
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
        rustup update
    } >> "$LOG_FILE" 2>&1
    success "Rust installed"
}

# Install Just
install_just() {
    if command_exists just; then
        info "Just already installed"
        return
    fi
    info "Installing Just command runner..."
    {
        curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to /usr/local/bin
    } >> "$LOG_FILE" 2>&1
    success "Just installed"
}

# Install CUDA Toolkit
install_cuda() {
    if is_package_installed "cuda-toolkit-12-9"; then
        info "CUDA Toolkit 12.9 already installed"
        return
    fi
    info "Installing CUDA Toolkit 12.9..."
    if ! check_dpkg_status; then
        exit $EXIT_DPKG_ERROR
    fi
    {
        distribution=$(grep '^ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"'| tr -d '\.')
        if ! wget https://developer.download.nvidia.com/compute/cuda/repos/$distribution/$(/usr/bin/uname -m)/cuda-keyring_1.1-1_all.deb 2>&1; then
            error "Failed to download CUDA keyring"
            exit $EXIT_DEPENDENCY_FAILED
        fi
        if ! dpkg -i cuda-keyring_1.1-1_all.deb 2>&1; then
            error "Failed to install CUDA keyring"
            rm cuda-keyring_1.1-1_all.deb
            exit $EXIT_DEPENDENCY_FAILED
        fi
        rm cuda-keyring_1.1-1_all.deb
        if ! apt-get update 2>&1; then
            error "Failed to update package list for CUDA"
            exit $EXIT_DEPENDENCY_FAILED
        fi
        if ! apt-get install -y cuda-toolkit-12-9 2>&1; then
            error "Failed to install CUDA Toolkit 12.9"
            if apt-get install -y cuda-toolkit-12-9 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        fi
        
        # Set up CUDA environment variables
        echo 'export PATH=/usr/local/cuda-12.9/bin:$PATH' >> ~/.bashrc
        echo 'export LD_LIBRARY_PATH=/usr/local/cuda-12.9/lib64:$LD_LIBRARY_PATH' >> ~/.bashrc
    } >> "$LOG_FILE" 2>&1
    success "CUDA Toolkit 12.9 installed"
}

# Install Rust dependencies
install_rust_deps() {
    info "Installing Rust dependencies..."

    # Source the Rust environment
    source "$HOME/.cargo/env" || {
        error "Failed to source $HOME/.cargo/env. Ensure Rust is installed."
        exit $EXIT_DEPENDENCY_FAILED
    }

    # Check and install cargo if not present
    if ! command_exists cargo; then
        if ! check_dpkg_status; then
            exit $EXIT_DPKG_ERROR
        fi
        info "Installing cargo..."
        apt update >> "$LOG_FILE" 2>&1 || {
            error "Failed to update package list for cargo"
            exit $EXIT_DEPENDENCY_FAILED
        }
        apt install -y cargo >> "$LOG_FILE" 2>&1 || {
            error "Failed to install cargo"
            if apt install -y cargo 2>&1 | grep -q "dpkg was interrupted"; then
                exit $EXIT_DPKG_ERROR
            fi
            exit $EXIT_DEPENDENCY_FAILED
        }
    fi

    # Always install rzup and the RISC Zero Rust toolchain
    info "Installing rzup..."
    curl -L https://risczero.com/install | bash >> "$LOG_FILE" 2>&1 || {
        error "Failed to install rzup"
        exit $EXIT_DEPENDENCY_FAILED
    }
    # Update PATH in the current shell
    export PATH="$PATH:/root/.risc0/bin"
    # Source bashrc to ensure environment is updated
    PS1='' source ~/.bashrc >> "$LOG_FILE" 2>&1 || {
        error "Failed to source ~/.bashrc after rzup install"
        exit $EXIT_DEPENDENCY_FAILED
    }
    # Install RISC Zero Rust toolchain
    rzup install rust >> "$LOG_FILE" 2>&1 || {
        error "Failed to install RISC Zero Rust toolchain"
        exit $EXIT_DEPENDENCY_FAILED
    }

    # Detect the RISC Zero toolchain
    TOOLCHAIN=$(rustup toolchain list | grep risc0 | head -1)
    if [ -z "$TOOLCHAIN" ]; then
        error "No RISC Zero toolchain found after installation"
        exit $EXIT_DEPENDENCY_FAILED
    fi
    info "Using RISC Zero toolchain: $TOOLCHAIN"

    # Install cargo-risczero
    if ! command_exists cargo-risczero; then
        info "Installing cargo-risczero..."
        cargo install cargo-risczero >> "$LOG_FILE" 2>&1 || {
            error "Failed to install cargo-risczero"
            exit $EXIT_DEPENDENCY_FAILED
        }
        rzup install cargo-risczero >> "$LOG_FILE" 2>&1 || {
            error "Failed to install cargo-risczero via rzup"
            exit $EXIT_DEPENDENCY_FAILED
        }
    fi

    # Install bento-client with the RISC Zero toolchain
    info "Installing bento-client..."
    RUSTUP_TOOLCHAIN=$TOOLCHAIN cargo install --locked --git https://github.com/risc0/risc0 bento-client --branch release-2.3 --bin bento_cli
 >> "$LOG_FILE" 2>&1 || {
        error "Failed to install bento-client"
        exit $EXIT_DEPENDENCY_FAILED
    }
    # Persist PATH for cargo binaries
    echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
    PS1='' source ~/.bashrc >> "$LOG_FILE" 2>&1 || {
        error "Failed to source ~/.bashrc after installing bento-client"
        exit $EXIT_DEPENDENCY_FAILED
    }

    # Install boundless-cli
    info "Installing boundless-cli..."
    cargo install --locked boundless-cli >> "$LOG_FILE" 2>&1 || {
        error "Failed to install boundless-cli"
        exit $EXIT_DEPENDENCY_FAILED
    }
    # Update PATH for boundless-cli
    export PATH="$PATH:/root/.cargo/bin"
    PS1='' source ~/.bashrc >> "$LOG_FILE" 2>&1 || {
        error "Failed to source ~/.bashrc after installing boundless-cli"
        exit $EXIT_DEPENDENCY_FAILED
    }

    success "Rust dependencies installed"
}


# Main installation flow
main() {
    echo -e "${BOLD}${CYAN}Boundless Prover Node Setup${RESET}"
    echo "========================================"
    mkdir -p "$(dirname "$LOG_FILE")"
    touch "$LOG_FILE"
    touch "$ERROR_LOG"
    echo "[START] Installation started at $(date)" >> "$LOG_FILE"
    echo "[START] Installation started at $(date)" >> "$ERROR_LOG"
    info "Logs will be saved to:"
    info "  - Full log: $LOG_FILE"
    info "  - Error log: $ERROR_LOG"
    echo
    if [[ $EUID -eq 0 ]]; then
        if [[ "$ALLOW_ROOT" == "true" ]]; then
            warning "Running as root (allowed via --allow-root)"
        else
            warning "Running as root user"
            read -e -p "Continue? (y/N): " response
            if [[ ! "$response" =~ ^[yY]$ ]]; then
                exit $EXIT_USER_ABORT
            fi
        fi
    else
        warning "This script requires root privileges or a user with appropriate permissions"
        info "Please ensure you have the necessary permissions to install packages and modify system settings"
    fi
    check_os
    update_system
    info "Installing all dependencies..."
    install_basic_deps
    install_gpu_drivers
    install_docker
    install_nvidia_toolkit
    install_rust
    install_just
    install_cuda
    install_rust_deps
    echo -e "\n${GREEN}${BOLD}Installation Complete!${RESET}"
    echo "[SUCCESS] Installation completed successfully at $(date)" >> "$LOG_FILE"

}

# Run main
main