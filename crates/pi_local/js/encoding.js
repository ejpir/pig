// Loaded before packages which construct TextEncoder/TextDecoder at module scope.
import { TextEncoder, TextDecoder } from 'text-encoding';
Object.assign(globalThis, { TextEncoder, TextDecoder });
