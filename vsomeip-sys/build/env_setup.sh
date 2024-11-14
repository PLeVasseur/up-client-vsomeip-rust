#!/bin/bash 

# console colors
RED='\033[0;31m'
GRN='\033[0;32m'
ORNG='\033[0;33m'
NC='\033[0;0m'

select_directory() {
	local directories=("$@")
	local selected_path=""

	# Check the number of directories
    if [ "${#directories[@]}" -eq 1 ]; then
        # If only one directory is found, set MY_PATH to it
        selected_path="${directories[0]}"
    else
        # Display directories to the user if more than one exists
        printf "Select a directory: \n" 1>&2
        for i in "${!directories[@]}"; do
            printf "%d) %s\n" "$((i + 1))" "${directories[i]}" 1>&2
        done

        # Prompt the user to enter a choice
        read -p "Enter the number of your choice: " choice

        # Validate the user’s input
        if [[ "$choice" -ge 1 && "$choice" -le "${#directories[@]}" ]]; then
            selected_path="${directories[choice - 1]}"
        else
            printf "${RED}Invalid selection ${NC}\n" 1>&2
            return 1
        fi
    fi

    # Return the selected path
    echo "$selected_path"
}

# find c++ include
if [ -d "/usr/include/c++/" ]; then
	CPP_DIRS=$( ls /usr/include/c++/ )
	CPP_ARRAY=($CPP_DIRS)
	STDLIB_DIR=$(select_directory "${CPP_ARRAY[@]}")
	if [ -z "$STDLIB_DIR" ]; then 
		return 1
	fi
	STDLIB_PATH="/usr/include/c++/${STDLIB_DIR}"
else
    # Print warning if the directory doesn't exist
    echo -e "${RED}/usr/include/c++/ does not exist.${NC}"
    return
fi

# find machine dir
MACHINE_NAME=$(uname -m)
MACHINE_DIRS=$( ls /usr/include/ | grep "${MACHINE_NAME}" )
MACHINE_ARRAY=($MACHINE_DIRS)
ARCH_DIR=$(select_directory "${MACHINE_ARRAY[@]}")
if [ -z "$ARCH_DIR" ]; then 
	return 1
fi
ARCH_PATH="/usr/include/${ARCH_DIR}"

# find arch c++ include
if [ -d "$ARCH_PATH/c++/" ]; then
	CPP_DIRS=$( ls ${ARCH_PATH}/c++/ )
	CPP_ARRAY=($CPP_DIRS)
	STDLIB_DIR=$(select_directory "${CPP_ARRAY[@]}")
	if [ -z "$STDLIB_DIR" ]; then 
		return 1
	fi
	ARCH_STDLIB_PATH="${ARCH_PATH}/c++/${STDLIB_DIR}"
else
    # Print warning if the directory doesn't exist
    echo -e "${RED}$ARCH_PATH/c++/ does not exist.${NC}"
    return
fi

# export the variables
export GENERIC_CPP_STDLIB_PATH=$STDLIB_PATH
export ARCH_SPECIFIC_CPP_STDLIB_PATH=$ARCH_STDLIB_PATH

echo -e "${ORNG}Set GENERIC_CPP_STDLIB_PATH=${GRN}$GENERIC_CPP_STDLIB_PATH${NC}"
echo -e "${ORNG}Set ARCH_SPECIFIC_CPP_STDLIB_PATH=${GRN}$ARCH_SPECIFIC_CPP_STDLIB_PATH${NC}"
