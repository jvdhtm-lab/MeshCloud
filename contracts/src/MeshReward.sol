// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title MeshReward
/// @notice Settlement contract for verified MeshCloud contributions.
///
/// Accepted contribution receipts are batched off-chain into a Merkle tree. The
/// authorized issuer submits only the Merkle root on-chain; recipients later
/// claim individual receipts with a Merkle proof. This keeps operational data
/// off-chain while making the settlement decision auditable, and it makes
/// duplicate claims impossible.
///
/// The leaf scheme matches the off-chain implementation exactly:
///   leaf = keccak256(abi.encodePacked(receiptHash, recipient, amount))
/// where `receiptHash = keccak256(bytes(receiptId))`, and internal nodes hash
/// the two children sorted (ascending) with odd nodes promoted.
contract MeshReward {
    /// Account permitted to submit batches.
    address public issuer;

    /// Batches whose root has been submitted.
    mapping(bytes32 => bool) public batchRoots;

    /// Receipts already claimed (keyed by receiptHash).
    mapping(bytes32 => bool) public claimed;

    /// Bookkeeping balances credited on claim.
    mapping(address => uint256) public balances;

    event IssuerChanged(address indexed previousIssuer, address indexed newIssuer);
    event BatchSubmitted(bytes32 indexed root, uint256 totalAmount, uint256 receiptCount);
    event Claimed(bytes32 indexed receiptHash, address indexed recipient, uint256 amount);

    error NotIssuer();
    error UnknownBatch();
    error AlreadyClaimed();
    error InvalidProof();
    error ZeroAddress();

    modifier onlyIssuer() {
        if (msg.sender != issuer) revert NotIssuer();
        _;
    }

    constructor(address initialIssuer) {
        if (initialIssuer == address(0)) revert ZeroAddress();
        issuer = initialIssuer;
        emit IssuerChanged(address(0), initialIssuer);
    }

    /// Change the authorized issuer.
    function setIssuer(address newIssuer) external onlyIssuer {
        if (newIssuer == address(0)) revert ZeroAddress();
        emit IssuerChanged(issuer, newIssuer);
        issuer = newIssuer;
    }

    /// Submit a batch root. Only the issuer may call this. Re-submitting an
    /// already-known root is a no-op.
    function submitBatch(bytes32 root, uint256 totalAmount, uint256 receiptCount) external onlyIssuer {
        if (!batchRoots[root]) {
            batchRoots[root] = true;
            emit BatchSubmitted(root, totalAmount, receiptCount);
        }
    }

    /// Claim a single receipt against a submitted batch root.
    function claim(
        bytes32 root,
        bytes32 receiptHash,
        address recipient,
        uint256 amount,
        bytes32[] calldata proof
    ) external {
        if (!batchRoots[root]) revert UnknownBatch();
        if (claimed[receiptHash]) revert AlreadyClaimed();

        bytes32 leaf = keccak256(abi.encodePacked(receiptHash, recipient, amount));
        if (!_verify(proof, root, leaf)) revert InvalidProof();

        claimed[receiptHash] = true;
        balances[recipient] += amount;
        emit Claimed(receiptHash, recipient, amount);
    }

    /// Verify a sorted-pair Merkle proof.
    function _verify(bytes32[] calldata proof, bytes32 root, bytes32 leaf) internal pure returns (bool) {
        bytes32 computed = leaf;
        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 sibling = proof[i];
            if (computed <= sibling) {
                computed = keccak256(abi.encodePacked(computed, sibling));
            } else {
                computed = keccak256(abi.encodePacked(sibling, computed));
            }
        }
        return computed == root;
    }
}
