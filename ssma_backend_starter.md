# SSMA Backend Starter

**Status**: Proposal  
**Date**: 2025-03-09

## Executive Summary

Create `ssma-backend-starter` - a collection of pre-built backend adapters and utilities that reduce boilerplate when connecting SSMA to common external services (Stripe, SES, etc.).

**Key insight**: Keep SSMA pure as a gateway, but provide reusable backend components that developers can copy or import.

---

## Problem Statement

### Current Developer Experience

Every project using SSMA must build a backend that implements the adapter interface:

```javascript
// Every project writes this boilerplate
app.post('/apply-intents', async (req, res) => {
  const { intents, context } = req.body;
  
  for (const intent of intents) {
    if (intent.name === 'payment/create') {
      // Call Stripe - custom implementation every time
      const result = await stripe.paymentIntents.create({...});
    }
    if (intent.name === 'email/send') {
      // Call SES - custom implementation every time
      await ses.sendEmail({...});
    }
    // ... more boilerplate
  }
});
```

### Pain Points

| Pain | Impact |
|------|--------|
| Repeated Stripe integration code | Hours of setup per project |
| Auth/JWT handling varies | Inconsistent security patterns |
| Database schema design | Decision fatigue, mistakes |
| Error handling differences | Debugging difficulty |
| No shared best practices | Each project reinvents patterns |

---

## Proposed Solution: SSMA Backend Starter

### Philosophy

```
┌─────────────────────────────────────────────────────────────┐
│                      SSMA Ecosystem                          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   CSMA Frontend ←→ SSMA Gateway ←→ YOUR BACKEND             │
│                                    ↑                         │
│                                    │                         │
│                           ssma-backend-starter               │
│                           (optional, copy-paste)             │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

**Not a framework** - a starter kit with:
- Pre-built adapter implementations
- Copy-paste or import as needed
- Opinionated but overridable
- Works with any Node.js/Rust backend

The starter should follow SSMA's canonical backend adapter contract:
- `context` is camelCase JSON
- auth identity lives in `context.user`
- adapters should not parse transport cookies directly

---

## Architecture

### Package Structure

```
packages/
  ssma-backend-starter/
    package.json
    README.md
    
    src/
      index.ts                    # Exports all adapters
      
      core/
        adapter.ts                # Base adapter interface
        router.ts                 # Express/Fastify/Hono router setup
        context.ts                # Request context helpers
        errors.ts                 # Standardized error types
        validation.ts             # Intent validation utilities
        
      adapters/
        index.ts                  # Export all adapters
        
        payment/
          stripe.ts               # Stripe adapter
          lemon-squeezy.ts        # LemonSqueezy adapter (future)
          
        email/
          ses.ts                  # Amazon SES adapter
          resend.ts               # Resend adapter
          postmark.ts             # Postmark adapter (future)
          
        storage/
          s3.ts                   # S3 file operations
          cloudflare-r2.ts        # R2 adapter (future)
          
        auth/
          jwt.ts                  # JWT token handling
          clerk.ts                # Clerk adapter (future)
          auth0.ts                # Auth0 adapter (future)
          
        database/
          prisma.ts               # Prisma integration
          drizzle.ts              # Drizzle integration
          
      templates/
        minimal/                  # Barebones backend
        full/                     # Full-featured starter
        rust/                     # Rust backend starter (Axum)
        
      schemas/
        intents/                  # Common intent schemas (Zod)
        responses/                # Response schemas
```

---

## Core Components

### 1. Base Adapter Interface

```typescript
// src/core/adapter.ts

import { z } from 'zod';

export interface SSMAContext {
  site: string;
  connectionId?: string;
  ip?: string;
  userAgent?: string;
  user?: {
    id?: string;
    role: string;
  } | null;
}

export interface IntentResult {
  id: string;
  status: 'acked' | 'rejected' | 'conflict' | 'failed';
  code?: string;
  message?: string;
  data?: unknown;
}

export interface AdapterConfig<TEnv = unknown> {
  name: string;
  env: TEnv;
}

export abstract class BaseAdapter<TConfig extends AdapterConfig = AdapterConfig> {
  constructor(protected config: TConfig) {}
  
  abstract readonly intentPrefix: string;
  abstract handle(
    intent: string,
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult>;
  
  protected ack(id: string, data?: unknown): IntentResult {
    return { id, status: 'acked', data };
  }
  
  protected reject(id: string, code: string, message: string): IntentResult {
    return { id, status: 'rejected', code, message };
  }
  
  protected conflict(id: string, message: string): IntentResult {
    return { id, status: 'conflict', message };
  }
  
  protected fail(id: string, message: string): IntentResult {
    return { id, status: 'failed', message };
  }
}
```

### 2. Router Helper

```typescript
// src/core/router.ts

import { Router, Request, Response } from 'express';
import { BaseAdapter, SSMAContext, IntentResult } from './adapter';

export interface SSMARouterConfig {
  adapters: BaseAdapter[];
  hooks?: {
    beforeApply?: (intents: unknown[], context: SSMAContext) => Promise<void>;
    afterApply?: (results: IntentResult[], context: SSMAContext) => Promise<void>;
  };
}

export function createSSMARouter(config: SSMARouterConfig): Router {
  const router = Router();
  
  // Build adapter map by prefix
  const adapterMap = new Map<string, BaseAdapter>();
  for (const adapter of config.adapters) {
    adapterMap.set(adapter.intentPrefix, adapter);
  }
  
  router.post('/apply-intents', async (req: Request, res: Response) => {
    const { intents, context } = req.body;
    const ssmaContext: SSMAContext = {
      site: context.site,
      connectionId: context.connectionId ?? null,
      ip: context.ip ?? req.ip,
      userAgent: context.userAgent ?? req.headers['user-agent'],
      user: context.user ?? null,
    };
    
    await config.hooks?.beforeApply?.(intents, ssmaContext);
    
    const results: IntentResult[] = [];
    
    for (const intent of intents) {
      const prefix = intent.intent.split('/')[0];
      const adapter = adapterMap.get(prefix);
      
      if (!adapter) {
        results.push({
          id: intent.id,
          status: 'rejected',
          code: 'UNKNOWN_INTENT',
          message: `No adapter for intent: ${intent.intent}`,
        });
        continue;
      }
      
      try {
        const result = await adapter.handle(
          intent.intent,
          intent.payload,
          ssmaContext
        );
        results.push(result);
      } catch (error) {
        results.push({
          id: intent.id,
          status: 'failed',
          message: error instanceof Error ? error.message : 'Unknown error',
        });
      }
    }
    
    await config.hooks?.afterApply?.(results, ssmaContext);
    
    res.json({ results });
  });
  
  router.post('/query/:name', async (req: Request, res: Response) => {
    // Query handling with adapter lookup
  });
  
  router.post('/subscribe', async (req: Request, res: Response) => {
    // Subscription handling
  });
  
  router.get('/health', (_req: Request, res: Response) => {
    res.json({ status: 'ok' });
  });
  
  return router;
}
```

---

## Pre-Built Adapters

### Stripe Adapter

```typescript
// src/adapters/payment/stripe.ts

import Stripe from 'stripe';
import { BaseAdapter, SSMAContext, IntentResult } from '../../core/adapter';
import { z } from 'zod';

const CreatePaymentSchema = z.object({
  amount: z.number().positive(),
  currency: z.string().length(3),
  customerId: z.string().optional(),
  metadata: z.record(z.string()).optional(),
});

const CreateCustomerSchema = z.object({
  email: z.string().email(),
  name: z.string().optional(),
  metadata: z.record(z.string()).optional(),
});

export interface StripeAdapterConfig {
  name: 'stripe';
  env: {
    secretKey: string;
    webhookSecret?: string;
  };
}

export class StripeAdapter extends BaseAdapter<StripeAdapterConfig> {
  readonly intentPrefix = 'payment';
  private stripe: Stripe;
  
  constructor(config: StripeAdapterConfig) {
    super(config);
    this.stripe = new Stripe(config.env.secretKey);
  }
  
  async handle(
    intent: string,
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const [prefix, action] = intent.split('/');
    
    switch (action) {
      case 'create':
        return this.createPayment(payload, context);
      case 'capture':
        return this.capturePayment(payload, context);
      case 'cancel':
        return this.cancelPayment(payload, context);
      case 'createCustomer':
        return this.createCustomer(payload, context);
      case 'updateCustomer':
        return this.updateCustomer(payload, context);
      case 'createSubscription':
        return this.createSubscription(payload, context);
      case 'cancelSubscription':
        return this.cancelSubscription(payload, context);
      default:
        return this.reject(
          '',
          'UNKNOWN_ACTION',
          `Unknown payment action: ${action}`
        );
    }
  }
  
  private async createPayment(
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const parsed = CreatePaymentSchema.safeParse(payload);
    if (!parsed.success) {
      return this.reject('', 'VALIDATION_ERROR', parsed.error.message);
    }
    
    const { amount, currency, customerId, metadata } = parsed.data;
    
    const paymentIntent = await this.stripe.paymentIntents.create({
      amount,
      currency,
      customer: customerId,
      metadata: {
        ...metadata,
        ssma_site: context.site,
        ssma_user: context.user?.id ?? 'guest',
      },
    });
    
    return this.ack('', {
      clientSecret: paymentIntent.client_secret,
      paymentIntentId: paymentIntent.id,
    });
  }
  
  private async createCustomer(
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const parsed = CreateCustomerSchema.safeParse(payload);
    if (!parsed.success) {
      return this.reject('', 'VALIDATION_ERROR', parsed.error.message);
    }
    
    const customer = await this.stripe.customers.create({
      ...parsed.data,
      metadata: {
        ssma_site: context.site,
        ssma_user: context.user?.id ?? 'guest',
      },
    });
    
    return this.ack('', { customerId: customer.id });
  }
  
  // ... other methods
}
```

### Email Adapter (Resend)

```typescript
// src/adapters/email/resend.ts

import { Resend } from 'resend';
import { BaseAdapter, SSMAContext, IntentResult } from '../../core/adapter';
import { z } from 'zod';

const SendEmailSchema = z.object({
  to: z.union([z.string().email(), z.array(z.string().email())]),
  subject: z.string(),
  html: z.string().optional(),
  text: z.string().optional(),
  from: z.string().optional(),
  replyTo: z.string().email().optional(),
  attachments: z.array(z.object({
    filename: z.string(),
    content: z.string(), // base64
  })).optional(),
});

export interface ResendAdapterConfig {
  name: 'resend';
  env: {
    apiKey: string;
    defaultFrom: string;
  };
}

export class ResendAdapter extends BaseAdapter<ResendAdapterConfig> {
  readonly intentPrefix = 'email';
  private resend: Resend;
  
  constructor(config: ResendAdapterConfig) {
    super(config);
    this.resend = new Resend(config.env.apiKey);
  }
  
  async handle(
    intent: string,
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const [prefix, action] = intent.split('/');
    
    switch (action) {
      case 'send':
        return this.send(payload, context);
      case 'sendTemplate':
        return this.sendTemplate(payload, context);
      default:
        return this.reject('', 'UNKNOWN_ACTION', `Unknown email action: ${action}`);
    }
  }
  
  private async send(
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const parsed = SendEmailSchema.safeParse(payload);
    if (!parsed.success) {
      return this.reject('', 'VALIDATION_ERROR', parsed.error.message);
    }
    
    const { to, subject, html, text, from, replyTo, attachments } = parsed.data;
    
    const { data, error } = await this.resend.emails.send({
      from: from || this.config.env.defaultFrom,
      to,
      subject,
      html,
      text,
      reply_to: replyTo,
      attachments: attachments?.map(a => ({
        filename: a.filename,
        content: a.content,
      })),
    });
    
    if (error) {
      return this.fail('', error.message);
    }
    
    return this.ack('', { messageId: data?.id });
  }
  
  private async sendTemplate(
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    // Template-based email sending with React Email support
  }
}
```

### S3 Storage Adapter

```typescript
// src/adapters/storage/s3.ts

import { S3Client, PutObjectCommand, GetObjectCommand, DeleteObjectCommand } from '@aws-sdk/client-s3';
import { getSignedUrl } from '@aws-sdk/s3-request-presigner';
import { BaseAdapter, SSMAContext, IntentResult } from '../../core/adapter';
import { z } from 'zod';

const UploadSchema = z.object({
  key: z.string(),
  contentType: z.string(),
  expiresIn: z.number().optional().default(3600),
});

const DownloadSchema = z.object({
  key: z.string(),
  expiresIn: z.number().optional().default(3600),
});

const DeleteSchema = z.object({
  key: z.string(),
});

export interface S3AdapterConfig {
  name: 's3';
  env: {
    region: string;
    bucket: string;
    accessKeyId: string;
    secretAccessKey: string;
  };
}

export class S3Adapter extends BaseAdapter<S3AdapterConfig> {
  readonly intentPrefix = 'storage';
  private client: S3Client;
  
  constructor(config: S3AdapterConfig) {
    super(config);
    this.client = new S3Client({
      region: config.env.region,
      credentials: {
        accessKeyId: config.env.accessKeyId,
        secretAccessKey: config.env.secretAccessKey,
      },
    });
  }
  
  async handle(
    intent: string,
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const [prefix, action] = intent.split('/');
    
    switch (action) {
      case 'getUploadUrl':
        return this.getUploadUrl(payload, context);
      case 'getDownloadUrl':
        return this.getDownloadUrl(payload, context);
      case 'delete':
        return this.delete(payload, context);
      default:
        return this.reject('', 'UNKNOWN_ACTION', `Unknown storage action: ${action}`);
    }
  }
  
  private async getUploadUrl(
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const parsed = UploadSchema.safeParse(payload);
    if (!parsed.success) {
      return this.reject('', 'VALIDATION_ERROR', parsed.error.message);
    }
    
    const { key, contentType, expiresIn } = parsed.data;
    
    // Prefix with site/user for isolation
    const subject = context.user?.id ?? 'guest';
    const prefixedKey = `${context.site}/${subject}/${key}`;
    
    const command = new PutObjectCommand({
      Bucket: this.config.env.bucket,
      Key: prefixedKey,
      ContentType: contentType,
    });
    
    const url = await getSignedUrl(this.client, command, { expiresIn });
    
    return this.ack('', { url, key: prefixedKey });
  }
  
  private async getDownloadUrl(
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const parsed = DownloadSchema.safeParse(payload);
    if (!parsed.success) {
      return this.reject('', 'VALIDATION_ERROR', parsed.error.message);
    }
    
    const command = new GetObjectCommand({
      Bucket: this.config.env.bucket,
      Key: parsed.data.key,
    });
    
    const url = await getSignedUrl(this.client, command, { 
      expiresIn: parsed.data.expiresIn 
    });
    
    return this.ack('', { url });
  }
  
  private async delete(
    payload: unknown,
    context: SSMAContext
  ): Promise<IntentResult> {
    const parsed = DeleteSchema.safeParse(payload);
    if (!parsed.success) {
      return this.reject('', 'VALIDATION_ERROR', parsed.error.message);
    }
    
    await this.client.send(new DeleteObjectCommand({
      Bucket: this.config.env.bucket,
      Key: parsed.data.key,
    }));
    
    return this.ack('', { deleted: true });
  }
}
```

---

## Usage Example

### Quick Start

```typescript
// backend/src/index.ts

import express from 'express';
import { createSSMARouter } from '@ssma/backend-starter';
import { StripeAdapter, ResendAdapter, S3Adapter } from '@ssma/backend-starter/adapters';

const app = express();
app.use(express.json());

const ssmaRouter = createSSMARouter({
  adapters: [
    new StripeAdapter({
      name: 'stripe',
      env: { secretKey: process.env.STRIPE_SECRET_KEY! },
    }),
    new ResendAdapter({
      name: 'resend',
      env: { 
        apiKey: process.env.RESEND_API_KEY!,
        defaultFrom: 'noreply@myapp.com',
      },
    }),
    new S3Adapter({
      name: 's3',
      env: {
        region: 'us-east-1',
        bucket: 'my-app-uploads',
        accessKeyId: process.env.AWS_ACCESS_KEY_ID!,
        secretAccessKey: process.env.AWS_SECRET_ACCESS_KEY!,
      },
    }),
  ],
  hooks: {
    beforeApply: async (intents, context) => {
      console.log(`Processing ${intents.length} intents for site ${context.site}`);
    },
  },
});

app.use('/ssma', ssmaRouter);

app.listen(3000, () => {
  console.log('SSMA Backend running on port 3000');
});
```

### Frontend Usage

```typescript
// CSMA frontend
import { createSSMAClient } from '@ssma/client';

const ssma = createSSMAClient({ url: 'ws://localhost:8080' });

// Create a payment
const result = await ssma.applyIntents([
  {
    intent: 'payment/create',
    payload: {
      amount: 2000, // $20.00
      currency: 'usd',
    },
  },
]);

// Send an email
await ssma.applyIntents([
  {
    intent: 'email/send',
    payload: {
      to: 'user@example.com',
      subject: 'Welcome!',
      html: '<h1>Welcome to our app!</h1>',
    },
  },
]);

// Get upload URL for file
const uploadResult = await ssma.applyIntents([
  {
    intent: 'storage/getUploadUrl',
    payload: {
      key: 'avatar.jpg',
      contentType: 'image/jpeg',
    },
  },
]);

// Upload file directly to S3
await fetch(uploadResult.results[0].data.url, {
  method: 'PUT',
  body: file,
  headers: { 'Content-Type': 'image/jpeg' },
});
```

---

## Template Projects

### Minimal Template

```
templates/minimal-backend/
  package.json
  tsconfig.json
  src/
    index.ts          # Basic Express + SSMA router
    .env.example
```

### Full Template

```
templates/full-backend/
  package.json
  tsconfig.json
  prisma/
    schema.prisma    # User, Session, etc.
  src/
    index.ts
    db.ts            # Prisma client
    auth/
      jwt.ts         # JWT utilities
      middleware.ts  # Auth middleware
    adapters/
      index.ts       # Import and configure adapters
      custom.ts      # Custom adapter example
    routes/
      ssma.ts        # SSMA routes
      auth.ts        # Auth routes
      webhooks/
        stripe.ts    # Stripe webhook handler
    .env.example
```

---

## Rust Version

For projects using the Rust runtime:

```rust
// src/main.rs

use axum::Router;
use ssma_backend_starter::{
    create_router, 
    adapters::{StripeAdapter, ResendAdapter},
    AdapterConfig,
};

#[tokio::main]
async fn main() {
    let stripe = StripeAdapter::new(StripeConfig {
        secret_key: std::env::var("STRIPE_SECRET_KEY").unwrap(),
    });
    
    let resend = ResendAdapter::new(ResendConfig {
        api_key: std::env::var("RESEND_API_KEY").unwrap(),
        default_from: "noreply@myapp.com".to_string(),
    });
    
    let ssma_router = create_router(vec![
        Box::new(stripe),
        Box::new(resend),
    ]);
    
    let app = Router::new()
        .nest("/ssma", ssma_router);
    
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

---

## Comparison: Plugin in SSMA vs Backend Starter

| Aspect | Plugin in SSMA | Backend Starter |
|--------|---------------|-----------------|
| Complexity | Adds to SSMA core | Separate package |
| Flexibility | Limited by plugin API | Full backend control |
| Security | API keys in gateway | API keys in backend |
| Customization | Plugin constraints | Copy and modify |
| Maintenance | SSMA team maintains | Community/you maintain |
| Adoption | Must upgrade SSMA | Drop-in to any backend |

---

## Implementation Roadmap

### Phase 1: Core (Week 1-2)
- [ ] `BaseAdapter` interface
- [ ] `createSSMARouter` helper
- [ ] Error types and utilities
- [ ] Minimal template

### Phase 2: Essential Adapters (Week 3-4)
- [ ] Stripe adapter (payments, subscriptions, webhooks)
- [ ] Resend adapter (email sending)
- [ ] S3 adapter (upload/download URLs)
- [ ] JWT auth utilities

### Phase 3: Extended Adapters (Week 5-6)
- [ ] Amazon SES adapter
- [ ] Postmark adapter
- [ ] Cloudflare R2 adapter
- [ ] LemonSqueezy adapter

### Phase 4: Templates (Week 7)
- [ ] Full template with Prisma
- [ ] Rust backend template
- [ ] Documentation site

---

## Recommendation

**Proceed with `ssma-backend-starter`** as a separate package, not a plugin system in SSMA.

**Benefits:**
1. Keeps SSMA focused on its core value (realtime sync + CRDT)
2. Provides immediate value to developers (copy-paste adapters)
3. Doesn't force architectural decisions on SSMA users
4. Easy to maintain and extend independently
5. Can be adopted incrementally (use one adapter, ignore others)

**Risk:**
- May need version sync with SSMA protocol changes
- Mitigation: Adapter interface is stable, version independently

---

## Next Steps

1. Create `packages/ssma-backend-starter/` directory
2. Implement core adapter interface and router
3. Build Stripe adapter as proof-of-concept
4. Create minimal template
5. Document usage patterns
