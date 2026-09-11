/**
 * The reference host against the contract.
 *
 * If this fails, the contract is wrong rather than the host — nothing else can
 * tell the difference between "no adapter satisfies this" and "this adapter is
 * broken".
 */

import { runConformance } from './support/conformance.js';
import { MemoryMailbox } from './support/mailbox.js';

runConformance('memory', () => new MemoryMailbox());
