// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {MeshReward} from "../src/MeshReward.sol";

contract MeshRewardTest is Test {
    MeshReward internal reward;
    address internal issuer = address(0xA11CE);
    address internal alice = address(0x1111);
    address internal bob = address(0x2222);

    function setUp() public {
        reward = new MeshReward(issuer);
    }

    function _leaf(bytes32 receiptHash, address recipient, uint256 amount) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(receiptHash, recipient, amount));
    }

    function _hashPair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a <= b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    function testSubmitAndClaim() public {
        bytes32 r1 = keccak256("rcpt-1");
        bytes32 r2 = keccak256("rcpt-2");
        bytes32 leaf1 = _leaf(r1, alice, 5);
        bytes32 leaf2 = _leaf(r2, bob, 3);
        bytes32 root = _hashPair(leaf1, leaf2);

        vm.prank(issuer);
        reward.submitBatch(root, 8, 2);

        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf2;
        reward.claim(root, r1, alice, 5, proof);

        assertEq(reward.balances(alice), 5);
        assertTrue(reward.claimed(r1));
    }

    function testCannotClaimTwice() public {
        bytes32 r1 = keccak256("rcpt-1");
        bytes32 leaf1 = _leaf(r1, alice, 5);
        bytes32 root = leaf1;

        vm.prank(issuer);
        reward.submitBatch(root, 5, 1);

        bytes32[] memory proof = new bytes32[](0);
        reward.claim(root, r1, alice, 5, proof);

        vm.expectRevert(MeshReward.AlreadyClaimed.selector);
        reward.claim(root, r1, alice, 5, proof);
    }

    function testOnlyIssuerCanSubmit() public {
        vm.expectRevert(MeshReward.NotIssuer.selector);
        reward.submitBatch(keccak256("root"), 1, 1);
    }

    function testInvalidProofRejected() public {
        bytes32 r1 = keccak256("rcpt-1");
        bytes32 leaf1 = _leaf(r1, alice, 5);

        vm.prank(issuer);
        reward.submitBatch(leaf1, 5, 1);

        bytes32[] memory proof = new bytes32[](0);
        vm.expectRevert(MeshReward.InvalidProof.selector);
        reward.claim(leaf1, r1, alice, 999, proof);
    }
}
