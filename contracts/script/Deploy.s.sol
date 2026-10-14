// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Script} from "forge-std/Script.sol";
import {MeshReward} from "../src/MeshReward.sol";

/// @notice Deploy MeshReward. The issuer address is read from the
/// `MESH_ISSUER_ADDRESS` environment variable.
contract DeployMeshReward is Script {
    function run() external returns (MeshReward) {
        address issuer = vm.envAddress("MESH_ISSUER_ADDRESS");
        vm.startBroadcast();
        MeshReward reward = new MeshReward(issuer);
        vm.stopBroadcast();
        return reward;
    }
}
