#!/bin/bash

# =============================================================================
# Script Name: user-setup.sh
# Description:
#   - Installs Rust programming language (in user directory).
#   - Installs Just command runner.
#   - Initializes git submodules.
#   - Installs RISC Zero toolchain.
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

# Function to safely reload environment variables
reload_environment() {
    info "Reloading environment variables..."
    
    # Method 1: Source cargo environment if exists
    if [[ -f "$HOME/.cargo/env" ]]; then
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env" 2>/dev/null || true
    fi
    
    # Method 2: Add known paths to current PATH
    local paths_to_add=(
        "$HOME/.cargo/bin"
        "$HOME/.local/bin"
        "$HOME/.rzup/bin"
    )
    
    for path_dir in "${paths_to_add[@]}"; do
        if [[ -d "$path_dir" && ":$PATH:" != *":$path_dir:"* ]]; then
            export PATH="$path_dir:$PATH"
            info "Added $path_dir to PATH"
        fi
    done
    
    # Method 3: Handle CUDA paths if they exist
    local cuda_paths=("/usr/local/cuda-13.0" "/usr/local/cuda")
    for cuda_path in "${cuda_paths[@]}"; do
        if [[ -d "$cuda_path/bin" ]]; then
            if [[ ":$PATH:" != *":$cuda_path/bin:"* ]]; then
                export PATH="$cuda_path/bin:$PATH"
            fi
            if [[ -d "$cuda_path/lib64" ]]; then
                if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
                    export LD_LIBRARY_PATH="$cuda_path/lib64:$LD_LIBRARY_PATH"
                else
                    export LD_LIBRARY_PATH="$cuda_path/lib64"
                fi
            fi
            break
        fi
    done
    
    # Small delay to ensure environment changes take effect
    sleep 1
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
        
        info "Updating Rust..."
        {
            rustup update
        } >> "$LOG_FILE" 2>&1
        success "Rust updated successfully."
    else
        info "Installing Rust programming language..."
        {
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        } >> "$LOG_FILE" 2>&1
        
        # Reload environment to get Rust tools in PATH
        reload_environment
        
        if command -v rustc &> /dev/null; then
            success "Rust installed successfully."
            info "Rust version: $(rustc --version)"
        else
            error "Rust installation failed. Cannot find rustc in PATH."
            exit 1
        fi
    fi
}

# Function to install the `just` command runner
install_just() {
    # Ensure environment is up to date
    reload_environment
    
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
        
        # Reload environment after cargo install
        reload_environment
        
        if command -v just &>/dev/null; then
            success "'just' installed via cargo successfully."
            return
        fi
    fi
    
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
    
    # Reload environment after installation
    reload_environment
    
    if command -v just &>/dev/null; then
        success "'just' installed successfully."
    else
        error "Failed to install 'just'"
        exit 1
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
            warning "Git submodule initialization failed or timed out after 5 minutes."
            warning "You can try manually running: git submodule update --init --recursive"
            warning "Continuing setup without submodule initialization."
        fi
    else
        warning "No .gitmodules file found. Skipping submodule initialization."
        warning "Make sure you're running this script from the project root directory."
    fi
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
            echo "export LD_LIBRARY_PATH=$cuda_path/lib64:\${LD_LIBRARY_PATH:-}"
        } >> ~/.bashrc
        success "CUDA paths added to ~/.bashrc"
    else
        info "CUDA paths already in ~/.bashrc"
    fi
    
    # Set for current session - handle empty LD_LIBRARY_PATH safely
    export PATH="$cuda_path/bin:$PATH"
    if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
        export LD_LIBRARY_PATH="$cuda_path/lib64:$LD_LIBRARY_PATH"
    else
        export LD_LIBRARY_PATH="$cuda_path/lib64"
    fi
    
    # Verify nvcc
    if command -v nvcc &> /dev/null; then
        success "NVCC found: $(nvcc --version | head -n1)"
    else
        error "NVCC not found even after setting PATH"
        return 1
    fi
}

# Function to install RISC Zero toolchain with improved error handling
install_risc_zero() {
    info "Installing RISC Zero toolchain..."
    
    # First reload environment to check if already installed
    reload_environment
    
    # Check if rzup is already installed
    if command -v rzup &> /dev/null; then
        info "RISC Zero toolchain (rzup) is already installed."
        info "Updating RISC Zero toolchain..."
        {
            rzup update 2>/dev/null || true
            rzup install
            rzup install risc0-groth16 2>/dev/null || true
        } >> "$LOG_FILE" 2>&1
        success "RISC Zero toolchain updated successfully."
        return 0
    fi

    info "Installing RISC Zero toolchain via risczero.com/install..."
    
    # Install RISC Zero toolchain
    {
        curl -L https://risczero.com/install | bash
    } >> "$LOG_FILE" 2>&1
    
    # Add RISC Zero to bashrc if not already there
    if ! grep -q "rzup\|\.rzup" ~/.bashrc 2>/dev/null; then
        echo "" >> ~/.bashrc
        echo "# RISC Zero toolchain" >> ~/.bashrc
        echo 'export PATH="$HOME/.rzup/bin:$PATH"' >> ~/.bashrc
        info "Added RISC Zero to ~/.bashrc"
    fi
    
    # Manually add to current PATH
    if [[ -d "$HOME/.rzup/bin" ]]; then
        export PATH="$HOME/.rzup/bin:$PATH"
        info "Added RISC Zero to current session PATH"
    fi
    
    # Reload environment multiple times with delays to ensure rzup is found
    for attempt in 1 2 3; do
        reload_environment
        sleep 2
        
        if command -v rzup &> /dev/null; then
            success "rzup found on attempt $attempt"
            break
        elif [[ $attempt -eq 3 ]]; then
            # Last attempt: check if binary exists and force add to PATH
            if [[ -x "$HOME/.risc0/bin/rzup" ]]; then
                export PATH="$HOME/.risc0/bin:$PATH"
                warning "rzup binary exists but required manual PATH addition"
            else
                error "rzup binary not found at $HOME/.risc/bin/rzup"
                warning "RISC Zero installation may have failed"
                return 1
            fi
        fi
    done
    
    # Install RISC Zero tools if rzup is available
    if command -v rzup &> /dev/null; then
        info "Installing RISC Zero components..."
        {
            # Install with timeout to prevent hanging
            timeout 900 rzup install || {
                warning "rzup install timed out but may have succeeded"
            }
            
            timeout 300 rzup install risc0-groth16 || {
                warning "risc0-groth16 installation failed, can be installed manually later"
            }
        } >> "$LOG_FILE" 2>&1
        
        success "RISC Zero toolchain installed successfully."
    else
        error "RISC Zero toolchain installation failed - rzup not accessible"
        warning "You may need to restart your shell and manually run:"
        warning "  rzup install"
        warning "  rzup install risc0-groth16"
        return 1
    fi
}


# Function to verify installations
verify_installations() {
    info "Verifying installations..."
    
    # Ensure environment is up to date before verification
    reload_environment
    
    local verification_passed=true
    
    # Check Rust
    if command -v rustc &> /dev/null && command -v cargo &> /dev/null; then
        success "✓ Rust: $(rustc --version)"
        success "✓ Cargo: $(cargo --version)"
    else
        error "✗ Rust/Cargo not found in PATH"
        verification_passed=false
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
        verification_passed=false
    fi
    
    # Check RISC Zero
    if command -v rzup &> /dev/null; then
        success "✓ RISC Zero toolchain: $(rzup --version 2>/dev/null || echo 'installed')"
    else
        error "✗ RISC Zero toolchain not found in PATH"
        if [[ -f "$HOME/.rzup/bin/rzup" ]]; then
            warning "rzup binary exists but not in PATH"
            info "Please restart your shell or run: exec \$SHELL"
        else
            warning "RISC Zero installation may have failed"
        fi
        verification_passed=false
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
            verification_passed=false
        fi
    done
    
    if [[ "$verification_passed" == "true" ]]; then
        success "All critical tools verified successfully!"
    else
        warning "Some tools failed verification. You may need to restart your shell."
    fi
}

display_final_instructions() {
    info ""
    info "=================================="
    info "Setup Complete!"
    info "=================================="
    info ""
    info "Next steps:"
    info "1. Restart your shell or run: exec \$SHELL"
    info "2. Verify installations: ./scripts/user-setup.sh --verify"
    info "3. If Docker group changes were made, you may need to log out and log back in"
    info ""
    info "Test your setup:"
    info "  - Rust: rustc --version && cargo --version"
    info "  - Just: just --version"
    info "  - RISC Zero: rzup --version"
    info "  - Docker: docker --version && docker ps"
    info "  - NVIDIA Docker: docker run --rm --gpus all nvidia/cuda:12.0-base-ubuntu20.04 nvidia-smi"
    info ""
    info "If any tools are not found, try:"
    info "  exec \$SHELL    # Restart shell to load new environment"
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

success "User setup completed successfully!"

info "===== User Setup Script Execution Ended at $(date) ====="

# Display final instructions
display_final_instructions

exit 0