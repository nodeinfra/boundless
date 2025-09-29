#!/bin/bash

# =============================================================================
# Script Name: system-setup.sh
# Description:
#   - Updates the system packages.
#   - Installs essential boundless packages (requires sudo).
#   - Installs GPU drivers for provers (requires sudo).
#   - Installs Docker with NVIDIA support (requires sudo).
#   - Installs CUDA Toolkit (requires sudo).
#   - Performs system cleanup (requires sudo).
#   - Verifies Docker with NVIDIA support.
#
# This script must be run with sudo privileges.
# After this script completes, run user-setup.sh as a regular user.
# =============================================================================

# Exit immediately if a command exits with a non-zero status,
# treat unset variables as an error, and propagate errors in pipelines.
set -euo pipefail

# =============================================================================
# Constants
# =============================================================================

SCRIPT_NAME="$(basename "$0")"
LOG_FILE="/var/log/${SCRIPT_NAME%.sh}.log"

# =============================================================================
# Functions
# =============================================================================

# Function to display informational messages
info() {
    printf "\e[34m[INFO]\e[0m %s\n" "$1"
}

# Function to display success messages
success() {
    printf "\e[32m[SUCCESS]\e[0m %s\n" "$1"
}

# Function to display error messages
error() {
    printf "\e[31m[ERROR]\e[0m %s\n" "$1" >&2
}

is_package_installed() {
    dpkg -s "$1" &> /dev/null
}

# Function to check if running with sudo
check_sudo() {
    if [[ $EUID -ne 0 ]]; then
        error "This script must be run with sudo privileges."
        error "Usage: sudo ./system-setup.sh"
        exit 1
    fi
}

# Function to check if the operating system is Ubuntu
check_os() {
    if [[ -f /etc/os-release ]]; then
        # Source the os-release file to get OS information
        # shellcheck source=/dev/null
        . /etc/os-release
        if [[ "${ID,,}" != "ubuntu" ]]; then
            error "Unsupported operating system: $NAME. This script is intended for Ubuntu."
            exit 1
        elif [[ "${VERSION_ID,,}" != "22.04" && "${VERSION_ID,,}" != "24.04" ]]; then
            error "Unsupported operating system version: $VERSION. This script is intended for Ubuntu 22.04 or 24.04."
            exit 1
        else
            info "Operating System: $PRETTY_NAME"
        fi
    else
        error "/etc/os-release not found. Unable to determine the operating system."
        exit 1
    fi
}

# Function to update and upgrade the system
update_system() {
    info "Updating and upgrading the system packages..."
    {
        apt update -y
        apt upgrade -y
    } >> "$LOG_FILE" 2>&1
    success "System packages updated and upgraded successfully."
}

# Function to install essential packages
install_packages() {
    local packages=(
        nvtop
        ubuntu-drivers-common
        build-essential
        libssl-dev
        curl
        gnupg
        ca-certificates
        lsb-release
        jq
    )

    info "Installing essential packages: ${packages[*]}..."
    {
        apt install -y "${packages[@]}"
    } >> "$LOG_FILE" 2>&1
    success "Essential packages installed successfully."
}

# Function to install specific GCC version for Ubuntu 22.04
install_gcc_version() {
    # Get Ubuntu version
    local ubuntu_version
    ubuntu_version=$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')

    if [[ "$ubuntu_version" == "22.04" ]]; then
        info "Installing GCC 12 for Ubuntu 22.04..."
        {
            apt install -y gcc-12 g++-12
            # Set gcc-12 and g++-12 as alternatives with higher priority
            update-alternatives --install /usr/bin/gcc gcc /usr/bin/gcc-12 100
            update-alternatives --install /usr/bin/g++ g++ /usr/bin/g++-12 100
        } >> "$LOG_FILE" 2>&1
        success "GCC 12 installed and configured successfully."
    else
        info "Skipping GCC 12 installation (not needed for Ubuntu $ubuntu_version)."
    fi
}

# Function to install CUDA Toolkit
install_cuda() {
    if is_package_installed "cuda-toolkit-13-0" && is_package_installed "nvidia-open"; then
        info "CUDA Toolkit and nvidia-open are already installed. Skipping CUDA installation."
    else
        info "Installing CUDA Toolkit and dependencies..."
        {
            # Get Ubuntu version for CUDA repository
            local ubuntu_version
            ubuntu_version=$(grep '^VERSION_ID=' /etc/os-release | cut -d'=' -f2 | tr -d '"')

            # Map Ubuntu versions to CUDA repository versions
            local cuda_repo_version
            case "$ubuntu_version" in
                "22.04")
                    cuda_repo_version="ubuntu2204"
                    ;;
                "24.04")
                    cuda_repo_version="ubuntu2404"
                    ;;
                *)
                    error "Unsupported Ubuntu version: $ubuntu_version"
                    exit 1
                    ;;
            esac

            info "Installing Nvidia CUDA keyring and repo for $cuda_repo_version"
            wget https://developer.download.nvidia.com/compute/cuda/repos/$cuda_repo_version/x86_64/cuda-keyring_1.1-1_all.deb
            dpkg -i cuda-keyring_1.1-1_all.deb
            rm cuda-keyring_1.1-1_all.deb
            apt-get update
            apt-get -y install cuda-toolkit-13-0 nvidia-open
        } >> "$LOG_FILE" 2>&1
        success "CUDA Toolkit installed successfully."
    fi
}

# Function to install Docker
install_docker() {
    if command -v docker &> /dev/null; then
        info "Docker is already installed. Skipping Docker installation."
    else
        info "Installing Docker..."
        {
            # Install prerequisites
            apt install -y apt-transport-https ca-certificates curl gnupg-agent software-properties-common

            # Add Docker's official GPG key
            curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /usr/share/keyrings/docker-archive-keyring.gpg

            # Set up the stable repository
            echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/docker-archive-keyring.gpg] https://download.docker.com/linux/ubuntu $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null

            # Update package index
            apt update -y

            # Install Docker Engine, CLI, and Containerd
            apt install -y docker-ce docker-ce-cli containerd.io

            # Enable Docker
            systemctl enable docker

            # Start Docker Service
            systemctl start docker

        } >> "$LOG_FILE" 2>&1
        success "Docker installed and started successfully."
    fi
}

# Function to add user to Docker group
add_user_to_docker_group() {
    local username="${SUDO_USER:-}"
    
    if [[ -z "$username" ]]; then
        error "Unable to determine the original user. Please run this script with sudo from a user account."
        exit 1
    fi

    if id -nG "$username" | grep -qw "docker"; then
        info "User '$username' is already in the 'docker' group."
    else
        info "Adding user '$username' to the 'docker' group..."
        {
            usermod -aG docker "$username"
        } >> "$LOG_FILE" 2>&1
        success "User '$username' added to the 'docker' group."
        info "To apply the new group membership, please log out and log back in."
    fi
}

# Function to install NVIDIA Container Toolkit
install_nvidia_container_toolkit() {
    info "Checking NVIDIA Container Toolkit installation..."

    # Check if already installed
    if command -v nvidia-ctk >/dev/null 2>&1 && is_package_installed "nvidia-container-toolkit"; then
        success "NVIDIA Container Toolkit is already installed."

        # Ensure runtime is configured
        info "Configuring NVIDIA Container Runtime..."
        nvidia-ctk runtime configure --runtime=docker
        systemctl restart docker
        return
    fi

    info "Installing NVIDIA Container Toolkit..."

    {
        # Configure the production repository
        curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey | gpg --dearmor -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg \
        && curl -s -L https://nvidia.github.io/libnvidia-container/stable/deb/nvidia-container-toolkit.list | \
            sed 's#deb https://#deb [signed-by=/usr/share/keyrings/nvidia-container-toolkit-keyring.gpg] https://#g' | \
            tee /etc/apt/sources.list.d/nvidia-container-toolkit.list

        # Update the packages list from the repository
        apt-get update -y

        # Install the NVIDIA Container Toolkit packages
        apt-get install -y nvidia-container-toolkit

        # Configure the container runtime
        nvidia-ctk runtime configure --runtime=docker

        # Restart the Docker daemon
        systemctl restart docker

    } >> "$LOG_FILE" 2>&1

    # Verify installation
    if command -v nvidia-ctk >/dev/null 2>&1; then
        success "NVIDIA Container Toolkit installed and configured successfully."
    else
        error "Failed to install NVIDIA Container Toolkit."
        return 1
    fi
}

# Function to configure Docker daemon for NVIDIA
configure_docker_nvidia() {
    info "Configuring Docker to use NVIDIA runtime by default..."

    {
        # Create Docker daemon configuration directory if it doesn't exist
        mkdir -p /etc/docker

        # Create or overwrite daemon.json with NVIDIA runtime configuration
        tee /etc/docker/daemon.json <<EOF
{
    "default-runtime": "nvidia",
    "runtimes": {
        "nvidia": {
            "path": "nvidia-container-runtime",
            "runtimeArgs": []
        }
    },
    "insecure-registries": ["172.30.0.12:5000"]
}
EOF

        # Restart Docker to apply the new configuration
        sudo systemctl restart docker
    } >> "$LOG_FILE" 2>&1

    success "Docker configured to use NVIDIA runtime by default."
}

# Function to perform system cleanup
cleanup() {
    info "Cleaning up unnecessary packages..."
    {
        apt autoremove -y
        apt autoclean -y
    } >> "$LOG_FILE" 2>&1
    success "Cleanup completed."
}

# =============================================================================
# Main Script Execution
# =============================================================================

# Check if running with sudo
check_sudo

# Redirect all output to log file
exec > >(tee -a "$LOG_FILE") 2>&1

# Display start message with timestamp
info "===== System Setup Script Execution Started at $(date) ====="

# Check if the operating system is Ubuntu
check_os

# Update and upgrade the system
update_system

# Install essential packages
install_packages

# Install specific GCC version for Ubuntu 22.04
install_gcc_version

# Install Docker
install_docker

# Add user to Docker group
add_user_to_docker_group

# Install NVIDIA Container Toolkit
install_nvidia_container_toolkit

# Configure Docker to use NVIDIA runtime
configure_docker_nvidia

# Install CUDA Toolkit
# install_cuda

# Cleanup
cleanup

success "System setup completed successfully!"

info "===== System Setup Script Execution Ended at $(date) ====="

info ""
info "Next step: Run the user setup script as a regular user:"
info "  ./user-setup.sh"
info ""
info "Note: You may need to log out and log back in for Docker group changes to take effect."

exit 0