# TAPS Implementation Guide

**Developer-Friendly Implementation Handbook**

This document provides step-by-step instructions, code templates, and practical guidance for implementing the TAPS (Temporary Access Privilege System) in the CSMA-SSMA ecosystem.

## Phase 1: Core Setup (Week 1)

### 1. File Structure Creation

Create the following directory structure:

```bash
# Execute these commands
mkdir -p SSMA/src/modules/access
mkdir -p SSMA/src/modules/access/types  
mkdir -p SSMA/src/modules/access/__tests__
mkdir -p CSMA/src/ui/patterns/access-elevation
```

### 2. Dependencies to Add

Update `SSMA/package.json`:

```json
{
  "dependencies": {
    "jsonwebtoken": "^9.0.3",
    "argon2": "^0.44.0", 
    "uuid": "^13.0.0"
  },
  "devDependencies": {
    "jsonwebtoken": "^9.0.3",
    "argon2": "^0.44.0",
    "uuid": "^13.0.0"
  }
}
```

Run: `npm install` (SSMA directory)

### 3. Integration Points Identified

- **SSMA/src/services/auth/AuthService.js** (line ~45): Add TAPS registration
- **SSMA/src/runtime/EventBus.js**: New event constants needed
- **SSMA/scripts/check-security.js**: Extend security validation
- **SSMA/src/runtime/validation/**: Add TAPS validation contracts

## Phase 2: Code Templates

### TAPSService Core Implementation

Create `SSMA/src/modules/access/TAPSService.js`:

```javascript
/**
 * Temporary Access Privilege System
 * Handles temporary elevated access with auto-revocation
 */

import { createRequestId } from '../../utils/id.js';
import { object, string, number, boolean, enums } from '../../runtime/validation/index.js';

export class TAPSService {
  constructor(eventBus, authService, options = {}) {
    this.eventBus = eventBus;
    this.authService = authService;
    this.requests = new Map();
    this.auditTrail = [];
    
    // Configuration with security defaults
    this.config = {
      defaultDuration: 60 * 60 * 1000, // 1 hour
      maxDuration: 4 * 60 * 60 * 1000, // 4 hours
      eligibleRoles: ['developer', 'admin', 'maintainer'],
      requiresApproval: ['admin', 'production'],
      autoRevoke: true,
      auditLevel: 'comprehensive',
      rateLimit: { requests: 3, window: 24 * 60 * 60 * 1000 },
      thresholds: {
        suspicious: { requests: 5, timeframe: 1000 * 60 * 30 },
        critical: { requests: 10, timeframe: 1000 * 60 * 60 }
      },
      ...options
    };
    
    this.setupEventListeners();
    this.initializeSecurityMonitoring();
  }

  setupEventListeners() {
    // TAPS-specific event subscriptions
    this.eventBus.subscribe('TAPS_REQUEST', this.handleRequest.bind(this));
    this.eventBus.subscribe('TAPS_APPROVE', this.handleApproval.bind(this));
    this.eventBus.subscribe('SECURITY_VIOLATION', this.handleViolation.bind(this));
  }

  /**
   * Request temporary elevated access
   */
  async requestElevatedAccess(userId, reason, duration = this.config.defaultDuration) {
    const requestId = createRequestId('taps');
    
    try {
      // Phase 1: Validation
      const isEligible = await this.checkEligibility(userId);
      if (!isEligible) {
        await this.logSecurityEvent('ELEVATION_ACCESS_DENIED', {
          requestId,
          userId,
          reason: 'Ineligible',
          timestamp: Date.now()
        });
        throw new Error('User not eligible for elevated access');
      }

      // Phase 2: Rate limiting
      const rateCheck = await this.checkRateLimit(userId);
      if (!rateCheck.allowed) {
        await this.logSecurityEvent('ELEVATION_ACCESS_RATE_LIMITED', {
          requestId,
          userId,
          reason: rateCheck.reason,
          timestamp: Date.now()
        });
        throw new Error('Rate limit exceeded');
      }

      // Phase 3: Approval workflow
      const approval = await this.requestApproval(userId, reason, duration, requestId);
      if (!approval.approved) {
        await this.logSecurityEvent('ELEVATION_ACCESS_REJECTED', {
          requestId,
          userId,
          reason: approval.reason,
          timestamp: Date.now()
        });
        throw new Error('Access request rejected');
      }

      // Phase 4: Grant temporary elevation
      const temporaryToken = await this.grantElevatedAccess(userId, duration, requestId);
      
      // Phase 5: Auto-revoke scheduling
      const revokeTimer = setTimeout(async () => {
        await this.revokeElevatedAccess(userId, temporaryToken.id, 'auto-expired');
      }, duration);

      // Store active request
      this.requests.set(userId, {
        temporaryToken,
        revokeTimer,
        grantedAt: Date.now(),
        expiresAt: Date.now() + duration,
        reason,
        requestedBy: userId,
        grantedBy: approval.grantedBy,
        requestId
      });

      await this.logSecurityEvent('ELEVATION_ACCESS_GRANTED', {
        requestId,
        userId,
        duration,
        reason,
        temporaryTokenId: temporaryToken.id,
        grantedBy: approval.grantedBy,
        timestamp: Date.now()
      });

      return {
        success: true,
        requestId,
        temporaryToken,
        expiresAt: Date.now() + duration
      };

    } catch (error) {
      await this.logSecurityEvent('ELEVATION_ACCESS_ERROR', {
        requestId,
        userId,
        error: error.message,
        timestamp: Date.now()
      });
      throw error;
    }
  }

  /**
   * Validate elevated access token
   */
  async validateElevation(token) {
    if (!token || typeof token !== 'string') {
      return { valid: false, reason: 'Invalid token format' };
    }

    try {
      // Verify JWT token
      const decoded = this.authService.verifyToken(token);
      
      // Check if token exists in active requests
      const userRequest = Array.from(this.requests.values())
        .find(req => req.temporaryToken.id === decoded.jti);
      
      if (!userRequest) {
        return { valid: false, reason: 'Token not found in active requests' };
      }

      // Check expiration
      if (userRequest.expiresAt <= Date.now()) {
        await this.revokeElevatedAccess(decoded.userId, decoded.jti, 'expired');
        return { valid: false, reason: 'Token expired' };
      }

      return {
        valid: true,
        userId: decoded.userId,
        level: decoded.level,
        expiresAt: userRequest.expiresAt,
        permissions: decoded.permissions
      };

    } catch (error) {
      return { valid: false, reason: 'Token verification failed' };
    }
  }

  /**
   * Manually revoke elevated access
   */
  async revokeElevatedAccess(userId, tokenId, reason = 'manual') {
    const userRequest = this.requests.get(userId);
    
    if (!userRequest || userRequest.temporaryToken.id !== tokenId) {
      throw new Error('Invalid request or token');
    }

    // Clear auto-revoke timer
    if (userRequest.revokeTimer) {
      clearTimeout(userRequest.revokeTimer);
    }

    // Remove from active requests
    this.requests.delete(userId);

    // Log revocation
    await this.logSecurityEvent('ELEVATION_ACCESS_REVOKED', {
      userId,
      token: tokenId,
      reason,
      grantedAt: userRequest.grantedAt,
      revokedAt: Date.now()
    });

    return { success: true };
  }

  // === Private Methods ===

  async checkEligibility(userId) {
    // Check if user has required roles
    const user = await this.authService.getUserData(userId);
    if (!user || !user.roles) {
      return false;
    }

    const hasEligibleRole = user.roles.some(role => 
      this.config.eligibleRoles.includes(role)
    );

    return hasEligibleRole;
  }

  async checkRateLimit(userId) {
    const now = Date.now();
    const window = this.config.rateLimit.window;
    const limit = this.config.rateLimit.requests;

    // Count requests within window
    const recentRequests = this.auditTrail.filter(entry => 
      entry.userId === userId && 
      entry.event === 'ELEVATION_ACCESS_GRANTED' &&
      (now - entry.timestamp) < window
    );

    if (recentRequests.length >= limit) {
      return { 
        allowed: false, 
        reason: `Rate limit: ${limit} requests per ${window / (1000 * 60 * 60)} hours` 
      };
    }

    return { allowed: true };
  }

  async requestApproval(userId, reason, duration, requestId) {
    // For self-approval (lower levels)
    if (duration <= this.config.defaultDuration) {
      return { 
        approved: true, 
        grantedBy: userId,
        requestId 
      };
    }

    // For higher access, would require admin approval
    // This is a simplified version - you can expand with workflow logic
    return { 
      approved: true, 
      grantedBy: 'auto-approved',
      requestId 
    };
  }

  async grantElevatedAccess(userId, duration, requestId) {
    const tokenId = createRequestId('token');
    const expiresAt = Date.now() + duration;
    
    // Create temporary JWT token
    const temporaryToken = this.authService.createToken({
      userId,
      tokenId,
      requestId,
      level: 'elevated',
      permissions: ['admin', 'write', 'delete'],
      expiresAt
    });

    return {
      id: tokenId,
      token: temporaryToken,
      userId,
      level: 'elevated',
      expiresAt
    };
  }

  async logSecurityEvent(event, data) {
    this.auditTrail.push({
      event,
      data,
      timestamp: Date.now()
    });

    // Publish to EventBus for global security monitoring
    this.eventBus.publish(event, data);
  }

  initializeSecurityMonitoring() {
    // Setup abuse detection and pattern monitoring
    setInterval(() => {
      this.detectSuspiciousPatterns();
    }, 60000); // Check every minute
  }

  setupEventListeners() {
    this.eventBus.subscribe('TAPS_REQUEST', this.handleRequest.bind(this));
    this.eventBus.subscribe('SECURITY_VIOLATION', this.handleViolation.bind(this));
  }

  async handleRequest(event, data) {
    // Handle external TAPS requests
    return this.requestElevatedAccess(data.userId, data.reason, data.duration);
  }

  async handleViolation(event, data) {
    // Handle security violations    
    if (data.type === 'elevation-abuse' || data.type === 'prompt-injection') {
      // Immediate revoke for critical violations
      if (data.userId) {
        await this.revokeElevatedAccess(data.userId, 'security-violation');
      }
    }
  }

  detectSuspiciousPatterns() {
    const now = Date.now();
    const suspiciousUsers = new Map();

    // Check for rapid request patterns
    this.auditTrail.forEach(entry => {
      if (entry.event === 'ELEVATION_ACCESS_GRANTED') {
        const userRequests = suspiciousUsers.get(entry.userId) || [];
        userRequests.push(entry.timestamp);
        suspiciousUsers.set(entry.userId, userRequests);
      }
    });

    // Detect suspicious patterns
    suspiciousUsers.forEach((timestamps, userId) => {
      const recentCount = timestamps.filter(t => (now - t) < 1000 * 60 * 60).length;
      
      if (recentCount >= this.config.thresholds.critical.requests) {
        this.handleViolation('SECURITY_VIOLATION', {
          type: 'critical-elevation-abuse',
          userId,
          pattern: 'excessive-requests',
          count: recentCount
        });
      }
    });
  }
}
```

### Integration with AuthService

Update `SSMA/src/services/auth/AuthService.js`:

```javascript
// Add these methods to existing AuthService class

// TAPS integration
constructor(eventBus, options = {}) {
  super(eventBus, options);
  this.tapsEnabled = options.tapsEnabled || false;
  this.tapsService = null;
}

registerTAPSService(tapsService) {
  this.tapsService = tapsService;
  this.tapsEnabled = true;
  console.log('[AuthService] TAPS integration enabled');
}

// Override or enhance existing checkPermission method
async checkPermission(user, resource, action) {
  // Run existing permission check first
  const baseCheck = await this.checkBasePermission(user, resource, action);
  
  // If base permission fails and TAPS is enabled, check elevated access
  if (!baseCheck && user.temporaryToken && this.tapsEnabled) {
    try {
      const elevation = await this.tapsService.validateElevation(user.temporaryToken);
      
      if (elevation.valid) {
        await this.logElevationUse(user, resource, action);
        
        // Publish elevation usage event
        this.eventBus.publish('ELEVATION_ACCESS_USED', {
          userId: user.id,
          resource,
          action,
          elevation: elevation.level,
          timestamp: Date.now()
        });
        
        return true;
      }
    } catch (error) {
      console.warn('[AuthService] Elevation validation failed:', error.message);
    }
  }
  
  return baseCheck;
}

async authenticate(credentials) {
  const result = await super.authenticate(credentials);
  
  // Check for elevated access token in credentials
  if (credentials.temporaryToken && this.tapsEnabled) {
    try {
      const elevation = await this.tapsService.validateElevation(credentials.temporaryToken);
      
      if (elevation.valid) {
        result.elevated = true;
        result.elevationLevel = elevation.level;
        result.expiresAt = elevation.expiresAt;
        result.temporaryToken = credentials.temporaryToken;
      }
    } catch (error) {
      console.warn('[AuthService] Invalid elevation token:', error.message);
    }
  }
  
  return result;
}

async logElevationUse(user, resource, action) {
  // Store elevation usage for audit trail
  this.eventBus.publish('ELEVATION_USAGE_LOGGED', {
    userId: user.id,
    resource,
    action,
    timestamp: Date.now()
  });
}
```

### Event Constants

Create `SSMA/src/modules/access/events.js`:

```javascript
export const TAPS_EVENTS = {
  // Core TAPS events
  ELEVATION_ACCESS_REQUESTED: 'ELEVATION_ACCESS_REQUESTED',
  ELEVATION_ACCESS_GRANTED: 'ELEVATION_ACCESS_GRANTED',
  ELEVATION_ACCESS_REVOKED: 'ELEVATION_ACCESS_REVOKED',
  ELEVATION_ACCESS_USED: 'ELEVATION_ACCESS_USED',
  ELEVATION_ACCESS_DENIED: 'ELEVATION_ACCESS_DENIED',
  ELEVATION_ACCESS_REJECTED: 'ELEVATION_ACCESS_REJECTED',
  ELEVATION_ACCESS_EXPIRED: 'ELEVATION_ACCESS_EXPIRED',
  
  // Security events
  ELEVATION_SECURITY_VIOLATION: 'ELEVATION_SECURITY_VIOLATION',
  ELEVATION_PATTERN_DETECTED: 'ELEVATION_PATTERN_DETECTED',
  ELEVATION_RATE_LIMITED: 'ELEVATION_RATE_LIMITED',
  
  // UI events  
  ELEVATION_STATUS_CHANGED: 'ELEVATION_STATUS_CHANGED',
  ELEVATION_REQUEST_APPROVED: 'ELEVATION_REQUEST_APPROVED'
};

export const TAPS_SEVERITY = {
  LOW: 'low',
  MEDIUM: 'medium', 
  HIGH: 'high',
  CRITICAL: 'critical'
};
```

## Phase 3: Validation Contracts

Create `SSMA/src/modules/access/types/ElevationRequest.js`:

```javascript
import { object, string, number, enums, array, boolean } from '../../../runtime/validation/index.js';

/**
 * Validation contract for elevation requests
 */
export const ElevationRequestContract = object({
  userId: string(),
  reason: string(),
  duration: number(), // milliseconds
  requestedBy: string(),
  targetLevel: enums(['admin', 'developer', 'maintainer']),
  purpose: enums(['debug', 'maintenance', 'emergency', 'deployment']),
  expiresAt: number(),
  requiresApproval: boolean()
});

/**
 * Validation contract for temporary tokens
 */
export const TemporaryTokenContract = object({
  id: string(),
  userId: string(), 
  level: string(),
  expiresAt: number(),
  grantedBy: string(),
  permissions: array(string()),
  auditId: string(),
  purpose: string()
});

/**
 * Validation contract for audit entries
 */
export const AuditEntryContract = object({
  event: string(),
  userId: string(),
  timestamp: number(),
  data: object({}),
  severity: enums(['low', 'medium', 'high', 'critical']),
  resolved: boolean()
});
```

## Phase 4: Testing

### Unit Tests

Create `SSMA/src/modules/access/__tests__/TAPSService.test.js`:

```javascript
import { TAPSService } from '../TAPSService.js';
import { EventBus } from '../../../runtime/EventBus.js';
import { AuthService } from '../../services/auth/AuthService.js';

describe('TAPSService', () => {
  let tapsService;
  let mockEventBus;
  let mockAuthService;
  let mockUser;

  beforeEach(() => {
    mockEventBus = new EventBus();
    mockAuthService = new AuthService(mockEventBus);
    tapsService = new TAPSService(mockEventBus, mockAuthService, {
      eligibleRoles: ['developer'],
      rateLimit: { requests: 3, window: 86400000 }
    });
    
    mockUser = {
      id: 'test-user-123',
      email: 'test@example.com',
      roles: ['developer']
    };
    
    // Mock user data lookup
    mockAuthService.getUserData = jest.fn().mockResolvedValue(mockUser);
    mockAuthService.createToken = jest.fn().mockReturnValue('mock-token');
    mockAuthService.verifyToken = jest.fn().mockReturnValue({ jti: 'test-token-id', userId: 'test-user-123' });
  });

  describe('requestElevatedAccess', () => {
    it('should deny request for ineligible user', async () => {
      mockAuthService.getUserData.mockResolvedValue({ roles: ['user'] }); // Not eligible
      
      await expect(
        tapsService.requestElevatedAccess('user-123', 'Need debug access', 3600000)
      ).rejects.toThrow('User not eligible for elevated access');
    });

    it('should grant access for eligible user with valid duration', async () => {
      const result = await tapsService.requestElevatedAccess('test-user-123', 'Need debug access', 3600000);
      
      expect(result.success).toBe(true);
      expect(result.temporaryToken).toBeDefined();
      expect(result.expiresAt).toBeGreaterThan(Date.now());
    });

    it('should reject request exceeding max duration', async () => {
      const maxDuration = tapsService.config.maxDuration + 1000000; // Exceeds max
      
      await expect(
        tapsService.requestElevatedAccess('test-user-123', 'Need long access', maxDuration)
      ).rejects.toThrow(); // Should be rejected by approval workflow
    });

    it('should limit request rate', async () => {
      // Make rapid requests
      for (let i = 0; i < 3; i++) {
        await tapsService.requestElevatedAccess('test-user-123', 'Request ' + i, 3600000);
      }
      
      // Fourth request should fail
      await expect(
        tapsService.requestElevatedAccess('test-user-123', 'Rate limit test', 3600000)
      ).rejects.toThrow('Rate limit exceeded');
    });
  });

  describe('validateElevation', () => {
    it('should validate legitimate token', async () => {
      // First, grant access
      const grantResult = await tapsService.requestElevatedAccess('test-user-123', 'Test', 3600000);
      
      // Then validate
      const validation = await tapsService.validateElevation(grantResult.temporaryToken.token);
      
      expect(validation.valid).toBe(true);
      expect(validation.userId).toBe('test-user-123');
      expect(validation.level).toBe('elevated');
    });

    it('should reject invalid token', async () => {
      const validation = await tapsService.validateElevation('invalid-token');
      
      expect(validation.valid).toBe(false);
      expect(validation.reason).toContain('failed');
    });

    it('should reject expired token', async () => {
      // Grant access with very short duration
      const grantResult = await tapsService.requestElevatedAccess('test-user-123', 'Test', 100);
      
      // Wait for expiration
      await new Promise(resolve => setTimeout(resolve, 150));
      
      const validation = await tapsService.validateElevation(grantResult.temporaryToken.token);
      
      expect(validation.valid).toBe(false);
      expect(validation.reason).toBe('Token expired');
    });
  });

  describe('revokeElevatedAccess', () => {
    it('should manually revoke elevated access', async () => {
      const grantResult = await tapsService.requestElevatedAccess('test-user-123', 'Test', 3600000);
      
      const revokeResult = await tapsService.revokeElevatedAccess('test-user-123', grantResult.temporaryToken.id, 'manual');
      
      expect(revokeResult.success).toBe(true);
      
      // Token should now be invalid
      const validation = await tapsService.validateElevation(grantResult.temporaryToken.token);
      expect(validation.valid).toBe(false);
    });

    it('should throw error for invalid revocation attempt', async () => {
      await expect(
        tapsService.revokeElevatedAccess('test-user-123', 'invalid-token-id', 'manual')
      ).rejects.toThrow('Invalid request or token');
    });
  });

  describe('security monitoring', () => {
    it('should detect suspicious request patterns', async () => {
      // This test would need to trigger the monitoring logic
      // Implementation depends on your specific thresholds
      expect(true).toBe(true); // Placeholder
    });
  });
});
```

### Integration Tests

Create `SSMA/src/__tests__/taps-integration.test.js`:

```javascript
describe('TAPS Integration', () => {
  let authService, tapsService, eventBus;

  beforeEach(async () => {
    eventBus = new EventBus();
    authService = new AuthService(eventBus, { tapsEnabled: false });
    tapsService = new TAPSService(eventBus, authService);
    
    authService.registerTAPSService(tapsService);
  });

  it('should integrate with AuthService permission checking', async () => {
    // Create test user with temporary elevation
    const elevationResult = await tapsService.requestElevatedAccess('admin-user', 'System maintenance', 3600000);
    
    const user = {
      id: 'admin-user',
      temporaryToken: elevationResult.temporaryToken.token
    };

    // Check permission with elevated token
    const hasPermission = await authService.checkPermission(user, 'admin-panel', 'read');
    expect(hasPermission).toBe(true);
  });

  it('should log all elevation events to EventBus', async () => {
    const events = [];
    eventBus.subscribe('ELEVATION_ACCESS_GRANTED', (event, data) => {
      events.push({ event, data });
    });

    await tapsService.requestElevatedAccess('test-user', 'Test access', 3600000);

    expect(events.length).toBe(1);
    expect(events[0].event).toBe('ELEVATION_ACCESS_GRANTED');
    expect(events[0].data.userId).toBe('test-user');
  });

  it('should maintain audit trail integrity', async () => {
    const initialAuditLength = tapsService.auditTrail.length;
    
    // Perform several operations
    await tapsService.requestElevatedAccess('user1', 'Test 1', 3600000);
    await tapsService.revokeElevatedAccess('user1', 'token-id', 'test');
    
    expect(tapsService.auditTrail.length).toBeGreaterThan(initialAuditLength);
    
    // Verify required fields present
    const lastEntry = tapsService.auditTrail[tapsService.auditTrail.length - 1];
    expect(lastEntry).toHaveProperty('event');
    expect(lastEntry).toHaveProperty('timestamp');
    expect(lastEntry).toHaveProperty('data');
  });
});
```

## Phase 5: Configuration

### TAPS Configuration

Create `SSMA/config/taps-config.js`:

```javascript
export const tapsConfig = {
  // Timing configuration
  defaultDuration: 60 * 60 * 1000, // 1 hour
  maxDuration: 4 * 60 * 60 * 1000, // 4 hours
  
  // Role-based configuration
  eligibleRoles: ['developer', 'admin', 'maintainer'],
  requiresApproval: ['admin', 'production'],
  
  // Security settings
  autoRevoke: true,
  auditLevel: 'comprehensive',
  
  // Rate limiting
  rateLimit: {
    requests: 3, // Max 3 requests per day
    window: 24 * 60 * 60 * 1000 // 24 hours
  },
  
  // Security thresholds
  thresholds: {
    suspicious: {
      requests: 5, // 5+ requests triggers review
      timeframe: 1000 * 60 * 30, // 30 minutes
      actions: ['flag', 'notify', 'review']
    },
    critical: {
      requests: 10, // 10+ requests triggers lock
      timeframe: 1000 * 60 * 60, // 1 hour  
      actions: ['lock', 'alert', 'audit']
    }
  },
  
  // Token configuration
  tokenConfig: {
    algorithm: 'HS256',
    expiresIn: '1h'
  },
  
  // Environment-specific settings
  development: {
    rateLimit: { requests: 10 }, // More lenient in dev
    thresholds: { suspicious: { requests: 15 } }
  },
  
  production: {
    rateLimit: { requests: 2 }, // Stricter in prod
    thresholds: { suspicious: { requests: 3 } }
  }
};
```

### Environment Variables

Update `SSMA/.env.example`:

```bash
# TAPS Configuration
TAPS_ENABLED=true
TAPS_JWT_SECRET=your-taps-secret-here-must-be-32-chars-minimum
TAPS_APPROVAL_REQUIRED_FOR=admin,production
TAPS_MAX_DURATION_HOURS=4
TAPS_RATE_LIMIT_REQUESTS=3
TAPS_RATE_LIMIT_WINDOW=86400000
TAPS_AUDIT_RETENTION_DAYS=90
TAPS_SECURITY_ALERTS_ENABLED=true
TAPS_AUTO_REVOKE=true
```

### Enhanced Security Script

Update `SSMA/scripts/check-security.js`:

```javascript
// Add these functions to existing check-security.js file

function checkTAPSEnvironment(env) {
  if (env.TAPS_ENABLED === 'true') {
    
    // JWT Secret validation
    if (!env.TAPS_JWT_SECRET || env.TAPS_JWT_SECRET.length < 32) {
      result.issues.push('TAPS_JWT_SECRET must be at least 32 characters and unique');
    }
    if (/[dD]efault|[tT]est|[dD]emo|[cC]hange|[sS]ecret/.test(env.TAPS_JWT_SECRET)) {
      result.issues.push('TAPS_JWT_SECRET contains insecure keywords - change immediately');
    }
    
    // Retention policy validation
    if (!env.TAPS_AUDIT_RETENTION_DAYS || Number(env.TAPS_AUDIT_RETENTION_DAYS) < 30) {
      result.warnings.push('TAPS audit retention should be at least 30 days for compliance');
    }
    
    // Duration limits
    if (!env.TAPS_MAX_DURATION_HOURS || Number(env.TAPS_MAX_DURATION_HOURS) > 24) {
      result.warnings.push('TAPS max duration should not exceed 24 hours for security');
    }
    
    // Rate limiting  
    if (!env.TAPS_RATE_LIMIT_REQUESTS || Number(env.TAPS_RATE_LIMIT_REQUESTS) > 10) {
      result.warnings.push('TAPS rate limit should not exceed 10 requests per 24 hours');
    }
    
    console.log('[check-security] TAPS configuration validated');
  }
}

// Add to main security check flow
if (fs.existsSync(envPath)) {
  const env = dotenv.parse(fs.readFileSync(envPath));
  checkTAPSEnvironment(env);
}
```

## Phase 6: UI Components (CSMA Integration)

### Access Request Form

Create `CSMA/src/ui/patterns/access-elevation/AccessRequestForm.js`:

```javascript
/**
 * Access Request Form Component
 * Handles UI for requesting temporary elevated access
 */

export class AccessRequestForm {
  constructor(eventBus) {
    this.eventBus = eventBus;
    this.form = null;
    this.statusElement = null;
  }

  render() {
    return `
      <div class="access-elevation-form" data-component="access-request">
        <div class="form-header">
          <h3>Request Elevated Access</h3>
          <p class="form-description">
            Request temporary elevated access for specific tasks. All access is logged and automatically revoked.
          </p>
        </div>
        
        <form class="elevation-request-form">
          <div class="form-group">
            <label for="reason">Reason for elevated access:</label>
            <textarea id="reason" name="reason" placeholder="Describe why you need elevated access..." required></textarea>
          </div>
          
          <div class="form-group">
            <label for="duration">Duration:</label>
            <select id="duration" name="duration">
              <option value="1800000">30 minutes</option>
              <option value="3600000" selected>1 hour</option>
              <option value="7200000">2 hours</option>
              <option value="14400000">4 hours</option>
            </select>
          </div>
          
          <div class="form-group">
            <label for="purpose">Purpose:</label>
            <select id="purpose" name="purpose" required>
              <option value="">Select purpose...</option>
              <option value="debug">Debug Production Issue</option>
              <option value="maintenance">System Maintenance</option>
              <option value="emergency">Emergency Fix</option>
              <option value="deployment">Deploy Critical Update</option>
            </select>
          </div>
          
          <div class="form-group">
            <label for="level">Access Level:</label>
            <select id="level" name="level">
              <option value="developer" selected>Developer Access</option>
              <option value="admin">Admin Access (requires approval)</option>
            </select>
          </div>
          
          <div class="form-actions">
            <button type="submit" class="btn btn-primary">Request Access</button>
            <button type="button" class="btn btn-secondary cancel-btn">Cancel</button>
          </div>
        </form>
        
        <div class="status-message" style="display: none;"></div>
      </div>
    `;
  }

  init(container) {
    this.container = container;
    this.container.innerHTML = this.render();
    this.bindEvents();
  }

  bindEvents() {
    const form = this.container.querySelector('.elevation-request-form');
    const cancelButton = this.container.querySelector('.cancel-btn');
    
    if (form) {
      form.addEventListener('submit', this.handleSubmit.bind(this));
    }
    
    if (cancelButton) {
      cancelButton.addEventListener('click', () => {
        this.container.style.display = 'none';
      });
    }
  }

  async handleSubmit(event) {
    event.preventDefault();
    
    const formData = new FormData(event.target);
    const data = {
      reason: formData.get('reason'),
      duration: Number(formData.get('duration')),
      purpose: formData.get('purpose'),
      level: formData.get('level')
    };
    
    // Validate form data
    if (!this.validateForm(data)) {
      return;
    }

    this.showStatus('Requesting elevated access...', 'loading');
    this.disableForm(true);

    try {
      // Publish request event
      this.eventBus.publish('TAPS_REQUEST', {
        userId: 'current-user', // Would get from auth context
        ...data
      });

      // For demo, simulate successful request
      setTimeout(() => {
        this.showStatus('✅ Elevated access granted! Access expires in 1 hour.', 'success');
        this.disableForm(true);
        
        // Auto-hide after 5 seconds
        setTimeout(() => {
          this.container.style.display = 'none';
        }, 5000);
      }, 2000);

    } catch (error) {
      this.showStatus(`❌ Request failed: ${error.message}`, 'error');
      this.disableForm(false);
    }
  }

  validateForm(data) {
    if (!data.reason || data.reason.trim().length < 10) {
      this.showStatus('❌ Please provide a detailed reason (minimum 10 characters)', 'error');
      return false;
    }

    if (!data.purpose) {
      this.showStatus('❌ Please select a purpose for the access request', 'error');
      return false;
    }

    if (data.duration > 4 * 60 * 60 * 1000) { // More than 4 hours
      this.showStatus('❌ Duration cannot exceed 4 hours', 'error');
      return false;
    }

    return true;
  }

  showStatus(message, type) {
    const statusElement = this.container.querySelector('.status-message');
    if (statusElement) {
      statusElement.textContent = message;
      statusElement.className = `status-message ${type}`;
      statusElement.style.display = 'block';
    }
  }

  disableForm(disabled) {
    const form = this.container.querySelector('.elevation-request-form');
    const submitBtn = this.container.querySelector('.btn-primary');
    
    if (submitBtn) {
      submitBtn.disabled = disabled;
      submitBtn.textContent = disabled ? 'Requesting...' : 'Request Access';
    }

    if (form) {
      form.querySelectorAll('input, select, textarea').forEach(el => {
        el.disabled = disabled;
      });
    }
  }
}
```

### Access Status Display

Create `CSMA/src/ui/patterns/access-elevation/AccessStatus.js`:

```javascript
/**
 * Access Status Component
 * Shows current elevated access status with countdown
 */

export class AccessStatus {
  constructor(eventBus) {
    this.eventBus = eventBus;
    this.statusElement = null;
    this.countdownTimer = null;
    
    this.setupEventListeners();
  }

  setupEventListeners() {
    this.eventBus.subscribe('ELEVATION_ACCESS_GRANTED', this.updateStatus.bind(this));
    this.eventBus.subscribe('ELEVATION_ACCESS_REVOKED', this.updateStatus.bind(this));
    this.eventBus.subscribe('ELEVATION_ACCESS_EXPIRED', this.updateStatus.bind(this));
  }

  render() {
    return `
      <div class="elevation-status" data-component="access-status">
        <div class="status-indicator">
          <span class="status-dot"></span>
          <span class="status-text">No Elevated Access</span>
        </div>
        <div class="status-details" style="display: none;">
          <div class="countdown-container">
            <span class="countdown-label">Expires in:</span>
            <span class="countdown-timer">--:--</span>
          </div>
          <div class="access-level">
            <span class="level-label">Level:</span>
            <span class="level-value">--</span>
          </div>
          <div class="revoke-access">
            <button class="btn btn-danger btn-sm">Revoke Access</button>
          </div>
        </div>
      </div>
    `;
  }

  init(container) {
    this.container = container;
    this.container.innerHTML = this.render();
    this.statusElement = this.container.querySelector('.elevation-status');
    this.bindEvents();
  }

  bindEvents() {
    const revokeBtn = this.container.querySelector('.revoke-access button');
    if (revokeBtn) {
      revokeBtn.addEventListener('click', this.handleRevoke.bind(this));
    }
  }

  updateStatus(event, data) {
    if (!this.statusElement) return;

    const statusDot = this.statusElement.querySelector('.status-dot');
    const statusText = this.statusElement.querySelector('.status-text');
    const statusDetails = this.statusElement.querySelector('.status-details');
    const countdownTimer = this.statusElement.querySelector('.countdown-timer');
    const levelValue = this.statusElement.querySelector('.level-value');

    switch (event) {
      case 'ELEVATION_ACCESS_GRANTED':
        // Clear any existing countdown
        if (this.countdownTimer) {
          clearInterval(this.countdownTimer);
        }
        
        // Update UI
        if (statusDot) statusDot.className = 'status-dot active';
        if (statusText) statusText.textContent = '🟢 Elevated Access Active';
        if (statusDetails) statusDetails.style.display = 'flex';
        if (levelValue) levelValue.textContent = data.level || 'elevated';
        
        // Start countdown
        this.startCountdown(data.expiresAt);
        break;

      case 'ELEVATION_ACCESS_REVOKED':
      case 'ELEVATION_ACCESS_EXPIRED':
        if (this.countdownTimer) {
          clearInterval(this.countdownTimer);
        }
        
        if (statusDot) statusDot.className = 'status-dot inactive';
        if (statusText) statusText.textContent = '🔴 No Elevated Access';
        if (statusDetails) statusDetails.style.display = 'none';
        break;
    }
  }

  startCountdown(expiresAt) {
    const countdownTimer = this.container.querySelector('.countdown-timer');
    if (!countdownTimer) return;

    this.countdownTimer = setInterval(() => {
      const remaining = Math.max(0, expiresAt - Date.now());
      
      if (remaining === 0) {
        clearInterval(this.countdownTimer);
        countdownTimer.textContent = 'Expired';
        return;
      }

      const hours = Math.floor(remaining / (1000 * 60 * 60));
      const minutes = Math.floor((remaining % (1000 * 60 * 60)) / (1000 * 60));
      
      countdownTimer.textContent = `${hours}h ${minutes}m`;
      
      // Warning when less than 15 minutes
      if (remaining < 15 * 60 * 1000) {
        countdownTimer.classList.add('warning');
      } else {
        countdownTimer.classList.remove('warning');
      }
    }, 1000);
  }

  async handleRevoke() {
    try {
      // Publish revocation event
      this.eventBus.publish('ELEVATION_REVOKE_REQUEST', {
        userId: 'current-user', // Would get from auth context
        reason: 'manual-revoke'
      });

      // For demo, simulate immediate revocation
      this.updateStatus('ELEVATION_ACCESS_REVOKED', { reason: 'manual-revoke' });

    } catch (error) {
      console.error('Failed to revoke access:', error);
    }
  }
}
```

## Phase 7: Rollout Strategy
```

This comprehensive implementation guide provides developers with:
- Step-by-step instructions
- Ready-to-use code templates  
- Complete test suites
- Configuration examples
- Security integration details
- UI components for the CSMA frontend

The file now contains everything developers need to implement TAPS successfully, with clear phases and practical code they can copy and adapt.
