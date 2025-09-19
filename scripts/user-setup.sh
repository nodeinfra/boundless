#!/bin/bash

# =============================================================================
# Script Name: user-setup.sh
# Description:
#   - Installs Rust programming language (in user directory).
#   - Installs Just command runner.
#   - Initializes git submodules.
#   - Verifies installations.
#
# This script should be run as a regular user (NOT with sudo).
# Run this after system-setup.sh has completed successfully.
# =============================================================================

# Exit immediately if a command exits with a non-zero status,
# treat unset variables as an error, and propagate errors in pipelines.
set -euo pipefail

# =============================================================================
# Constants
# =============================================================================

SCRIPT_NAME="$(basename "$0")"
LOG_FILE="$HOME/${SCRIPT_NAME%.sh}.log"

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

# Function to display warning messages
warning() {
    printf "\e[33m[WARNING]\e[0m %s\n" "$1"
}

# Function to check if not running with sudo
check_not_sudo() {
    if [[ $EUID -eq 0 ]] || [[ -n "${SUDO_USER:-}" ]]; then
        error "This script should NOT be run with sudo privileges."
        error "Please run as a regular user: ./user-setup.sh"
        exit 1
    fi
}

# Function to install Rust
install_rust() {
    if command -v rustc &> /dev/null; then
        info "Rust is already installed. Current version:"
        rustc --version
        
        # Ask if user wants to update
        if [[ -t 0 ]]; then
            read -rp "Do you want to update Rust? (y/N): " UPDATE_RUST
            case "$UPDATE_RUST" in
                [yY][eE][sS]|[yY])
                    info "Updating Rust..."
                    {
                        rustup update
                    } >> "$LOG_FILE" 2>&1
                    success "Rust updated successfully."
                    ;;
                *)
                    info "Skipping Rust update."
                    ;;
            esac
        else
            info "Skipping Rust installation (already installed)."
        fi
    else
        info "Installing Rust programming language..."
        {
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        } >> "$LOG_FILE" 2>&1
        
        # Source Rust environment variables for the current session
        if [[ -f "$HOME/.cargo/env" ]]; then
            # shellcheck source=/dev/null
            source "$HOME/.cargo/env"
            success "Rust installed successfully."
            info "Rust version: $(rustc --version)"
        else
            error "Rust installation failed. ~/.cargo/env not found."
            exit 1
        fi
    fi
}

# Function to install the `just` command runner
install_just() {
    if command -v just &>/dev/null; then
        info "'just' is already installed. Current version:"
        just --version
        return
    fi

    info "Installing the 'just' command-runner..."
    
    # Try to install via cargo first if Rust is available
    if command -v cargo &>/dev/null; then
        info "Installing 'just' via cargo..."
        {
            cargo install just
        } >> "$LOG_FILE" 2>&1
        success "'just' installed via cargo successfully."
    else
        # Fallback to downloading prebuilt binary to user's local bin
        info "Installing 'just' via prebuilt binary to ~/.local/bin..."
        {
            # Create ~/.local/bin if it doesn't exist
            mkdir -p "$HOME/.local/bin"
            
            # Download and install just
            curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh \
                | bash -s -- --to "$HOME/.local/bin"
            
            # Add ~/.local/bin to PATH if not already there
            if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
                echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc"
                export PATH="$HOME/.local/bin:$PATH"
                info "Added ~/.local/bin to PATH in ~/.bashrc"
            fi
        } >> "$LOG_FILE" 2>&1
        success "'just' installed successfully."
    fi
}

# Function to initialize git submodules
init_git_submodules() {
    if [[ -f ".gitmodules" ]]; then
        info "Initializing git submodules..."
        info "This may take a while depending on submodule sizes..."
        
        # Show progress and use timeout to prevent hanging
        if timeout 300 git submodule update --init --recursive --progress; then
            success "Git submodules initialized successfully."
        else
            error "Git submodule initialization failed or timed out after 5 minutes."
            warning "You can try manually running: git submodule update --init --recursive"
            
            # Ask user if they want to continue without submodules
            if [[ -t 0 ]]; then
                read -rp "Continue setup without initializing submodules? (y/N): " CONTINUE
                case "$CONTINUE" in
                    [yY][eE][sS]|[yY])
                        warning "Continuing setup without submodule initialization."
                        ;;
                    *)
                        error "Setup aborted. Please resolve submodule issues and retry."
                        exit 1
                        ;;
                esac
            else
                warning "Continuing setup without submodule initialization (non-interactive mode)."
            fi
        fi
    else
        warning "No .gitmodules file found. Skipping submodule initialization."
        warning "Make sure you're running this script from the project root directory."
    fi
}

# Function to verify Docker access
verify_docker_access() {
    info "Verifying Docker access..."
    
    if ! command -v docker &> /dev/null; then
        warning "Docker is not installed or not in PATH."
        return
    fi
    
    if docker ps &> /dev/null; then
        success "Docker access verified successfully."
        
        # Test NVIDIA Docker if available
        if docker run --rm --gpus all nvidia/cuda:12.0-base-ubuntu20.04 nvidia-smi &> /dev/null; then
            success "NVIDIA Docker runtime verified successfully."
        else
            warning "NVIDIA Docker runtime test failed. This is normal if you don't have NVIDIA GPUs or drivers installed."
        fi
    else
        warning "Cannot access Docker. You may need to:"
        warning "1. Log out and log back in to apply group membership changes"
        warning "2. Restart the Docker service: sudo systemctl restart docker"
        warning "3. Check if you're in the docker group: groups"
    fi
}

# Function to verify installations
verify_installations() {
    info "Verifying installations..."
    
    # Check Rust
    if command -v rustc &> /dev/null && command -v cargo &> /dev/null; then
        success "✓ Rust: $(rustc --version)"
        success "✓ Cargo: $(cargo --version)"
    else
        error "✗ Rust/Cargo not found in PATH"
    fi
    
    # Check Just
    if command -v just &> /dev/null; then
        success "✓ Just: $(just --version)"
    else
        error "✗ Just not found in PATH"
        if [[ -f "$HOME/.local/bin/just" ]]; then
            info "Just is installed in ~/.local/bin/just but not in PATH"
            info "Please restart your shell or run: source ~/.bashrc"
        fi
    fi
    
    # Check RISC Zero
    if command -v rzup &> /dev/null; then
        success "✓ RISC Zero toolchain: $(rzup --version 2>/dev/null || echo 'installed')"
    else
        warning "✗ RISC Zero toolchain not found in PATH"
        info "Please restart your shell to load ~/.bashrc updates"
    fi
    
    # Check CUDA/NVCC
    if command -v nvcc &> /dev/null; then
        success "✓ NVCC: $(nvcc --version | head -n1 | cut -d',' -f2 | tr -d ' ')"
    else
        warning "✗ NVCC not found in PATH"
        info "RISC Zero GPU acceleration will not work without NVCC"
    fi
    
    # Check essential system tools
    local tools=("gcc" "g++" "make" "curl" "git")
    for tool in "${tools[@]}"; do
        if command -v "$tool" &> /dev/null; then
            success "✓ $tool is available"
        else
            warning "✗ $tool not found"
        fi
    done
}

# Function to setup CUDA environment
setup_cuda_environment() {
    info "Setting up CUDA environment..."
    
    # Find CUDA installation
    local cuda_paths=(/usr/local/cuda-13.0 /usr/local/cuda)
    local cuda_path=""
    
    for path in "${cuda_paths[@]}"; do
        if [[ -d "$path/bin" ]]; then
            cuda_path="$path"
            break
        fi
    done
    
    if [[ -z "$cuda_path" ]]; then
        warning "CUDA installation not found in standard locations."
        warning "RISC Zero GPU acceleration may not work."
        return 1
    fi
    
    info "Found CUDA at: $cuda_path"
    
    # Check if already in bashrc
    if ! grep -q "cuda.*bin" ~/.bashrc 2>/dev/null; then
        info "Adding CUDA paths to ~/.bashrc..."
        {
            echo ""
            echo "# CUDA paths"
            echo "export PATH=$cuda_path/bin:\$PATH"
            echo "export LD_LIBRARY_PATH=$cuda_path/lib64:\$LD_LIBRARY_PATH"
        } >> ~/.bashrc
        success "CUDA paths added to ~/.bashrc"
    else
        info "CUDA paths already in ~/.bashrc"
    fi
    
    # Set for current session
    export PATH="$cuda_path/bin:$PATH"
    export LD_LIBRARY_PATH="$cuda_path/lib64:$LD_LIBRARY_PATH"
    
    # Verify nvcc
    if command -v nvcc &> /dev/null; then
        success "NVCC found: $(nvcc --version | head -n1)"
    else
        error "NVCC not found even after setting PATH"
        return 1
    fi
}
install_risc_zero() {
    info "Installing RISC Zero toolchain..."
    
    # Check if rzup is already installed
    if command -v rzup &> /dev/null; then
        info "RISC Zero toolchain (rzup) is already installed."
        
        # Check current version and offer to update
        if [[ -t 0 ]]; then
            read -rp "Do you want to update RISC Zero toolchain? (y/N): " UPDATE_RISC0
            case "$UPDATE_RISC0" in
                [yY][eE][sS]|[yY])
                    info "Updating RISC Zero toolchain..."
                    {
                        rzup update
                        rzup install
                        rzup install risc0-groth16
                    } >> "$LOG_FILE" 2>&1
                    success "RISC Zero toolchain updated successfully."
                    ;;
                *)
                    info "Skipping RISC Zero toolchain update."
                    ;;
            esac
        else
            info "Skipping RISC Zero toolchain installation (already installed)."
        fi
    else
        info "Installing RISC Zero toolchain via risczero.com/install..."
        {
            # Install RISC Zero toolchain
            curl -L https://risczero.com/install | bash
            
            # Source bashrc to get rzup in PATH
            source ~/.bashrc
            
            # Install RISC Zero tools
            rzup install
            rzup install risc0-groth16
        } >> "$LOG_FILE" 2>&1
        
        # Verify installation
        if command -v rzup &> /dev/null; then
            success "RISC Zero toolchain installed successfully."
            
            # Source bashrc for current session
            if [[ -f "$HOME/.bashrc" ]]; then
                source "$HOME/.bashrc"
            fi
        else
            error "RISC Zero toolchain installation failed."
            warning "You may need to restart your shell and run: rzup install"
            return 1
        fi
    fi
}
display_final_instructions() {
    info ""
    info "=================================="
    info "Setup Complete!"
    info "=================================="
    info ""
    info "Next steps:"
    info "1. Restart your shell or run: source ~/.bashrc"
    info "2. Verify installations: ./user-setup.sh --verify"
    info "3. If Docker group changes were made, you may need to log out and log back in"
    info ""
    info "Test your setup:"
    info "  - Rust: rustc --version && cargo --version"
    info "  - Just: just --version"
    info "  - RISC Zero: rzup --version"
    info "  - Docker: docker --version && docker ps"
    info "  - NVIDIA Docker: docker run --rm --gpus all nvidia/cuda:12.0-base-ubuntu20.04 nvidia-smi"
    info ""
}

# =============================================================================
# Main Script Execution
# =============================================================================

# Handle command line arguments
if [[ "${1:-}" == "--verify" ]]; then
    verify_installations
    verify_docker_access
    exit 0
fi

# Check if not running with sudo
check_not_sudo

# Redirect all output to log file
exec > >(tee -a "$LOG_FILE") 2>&1

# Display start message with timestamp
info "===== User Setup Script Execution Started at $(date) ====="

# Initialize git submodules
init_git_submodules

# Install Rust
install_rust

# Install Just
install_just

# Setup CUDA environment
setup_cuda_environment

# Install RISC Zero toolchain
install_risc_zero

# Verify installations
verify_installations

# Verify Docker access
verify_docker_access

success "User setup completed successfully!"

info "===== User Setup Script Execution Ended at $(date) ====="

# Display final instructions
display_final_instructions

exit 0