# Media Files Support in KalamDB

**Feature**: Media File Support (Images, Documents, Audio, Video)  
**Date**: 2025-10-13  
**Status**: Phase 1 Design Extension

## Overview

KalamDB now supports media files (images, documents, audio, video, etc.) alongside text messages. Media files use the same `contentRef` mechanism as large text messages, storing files once in shared storage and duplicating only lightweight references across participant storage partitions.

---

## Supported Media Types

### Images
- **Formats**: JPEG, PNG, GIF, WebP, HEIC, BMP, TIFF
- **Features**: Automatic thumbnail generation, dimension extraction
- **Max Size**: 50 MB (configurable)

### Documents
- **Formats**: PDF, DOCX, XLSX, PPTX, TXT, RTF, ODT, ODS
- **Features**: File size and page count extraction (PDF)
- **Max Size**: 50 MB (configurable)

### Audio
- **Formats**: MP3, WAV, OGG, M4A, FLAC, AAC, OPUS
- **Features**: Duration extraction, bitrate detection
- **Max Size**: 50 MB (configurable)

### Video
- **Formats**: MP4, MOV, AVI, WebM, MKV, FLV
- **Features**: Duration, resolution, thumbnail generation (first frame)
- **Max Size**: 100 MB (configurable)

### Archives
- **Formats**: ZIP, RAR, 7z, TAR, TAR.GZ, TAR.BZ2
- **Features**: File count and total size extraction
- **Max Size**: 50 MB (configurable)

---

## Architecture

### Storage Strategy

**Principle**: Media files ALWAYS use shared storage (regardless of size)

```
┌─────────────────────────────────────────────────────────┐
│  userA sends image to convA (3 participants)            │
└─────────────────────────────────────────────────────────┘
                              ↓
        ┌─────────────────────┴─────────────────────┐
        ↓                     ↓                      ↓
┌──────────────┐      ┌──────────────┐      ┌──────────────┐
│ userA/       │      │ userB/       │      │ userC/       │
│ batch-*.     │      │ batch-*.     │      │ batch-*.     │
│ parquet      │      │ parquet      │      │ parquet      │
│              │      │              │      │              │
│ msgId: 123   │      │ msgId: 123   │      │ msgId: 123   │
│ content:     │      │ content:     │      │ content:     │
│  "Check..."  │      │  "Check..."  │      │  "Check..."  │
│ contentRef:  │      │ contentRef:  │      │ contentRef:  │
│  s3://.../   │      │  s3://.../   │      │  s3://.../   │
│  media-123.  │      │  media-123.  │      │  media-123.  │
│  jpg         │      │  jpg         │      │  jpg         │
│ metadata:    │      │ metadata:    │      │ metadata:    │
│  thumbnail,  │      │  thumbnail,  │      │  thumbnail,  │
│  dimensions  │      │  dimensions  │      │  dimensions  │
└──────────────┘      └──────────────┘      └──────────────┘
                              ↓
                    ┌─────────────────────┐
                    │ Shared Storage      │
                    │ s3://bucket/shared/ │
                    │ conversations/      │
                    │ convA/              │
                    │ media-123.jpg       │
                    │ (2.4 MB, stored 1x) │
                    └─────────────────────┘
```

### Message Flow

#### 1. Upload Media

```http
POST /api/v1/messages/upload
Content-Type: multipart/form-data

conversationId: conv_abc123
file: vacation-photo.jpg (2.4 MB)
```

**Server Processing**:
1. Validate file type and size
2. Generate unique msgId (snowflake)
3. Upload to shared storage: `shared/conversations/convA/media-{msgId}.jpg`
4. Extract metadata:
   - Content type: `image/jpeg`
   - Dimensions: 1920x1080
   - File size: 2,457,600 bytes
5. Generate thumbnail (200x200 max, base64-encoded)
6. Return response:

```json
{
  "contentRef": "s3://bucket/shared/conversations/convA/media-1234567890123456.jpg",
  "contentType": "image/jpeg",
  "fileSize": 2457600,
  "thumbnail": "data:image/jpeg;base64,/9j/4AAQSkZJRg...",
  "metadata": {
    "width": 1920,
    "height": 1080
  }
}
```

#### 2. Submit Message with Media

```http
POST /api/v1/messages
Content-Type: application/json

{
  "conversationId": "conv_abc123",
  "from": "user_john",
  "timestamp": 1696800000000000,
  "content": "Check out this vacation photo!",
  "contentRef": "s3://bucket/.../media-1234567890123456.jpg",
  "metadata": {
    "role": "user",
    "contentType": "image/jpeg",
    "fileName": "vacation-photo.jpg",
    "fileSize": 2457600,
    "width": 1920,
    "height": 1080,
    "thumbnail": "data:image/jpeg;base64,/9j/4AAQ..."
  }
}
```

**Server Processing**:
1. Lookup conversation participants (userA, userB, userC)
2. Write message to RocksDB for each participant (parallel):
   - `userA:1234567890123456` → Message with contentRef
   - `userB:1234567890123456` → Message with contentRef
   - `userC:1234567890123456` → Message with contentRef
3. Acknowledge: `{"msgId": 1234567890123456, "acknowledged": true}`
4. Broadcast via WebSocket to subscribed users

#### 3. Query Messages (includes media)

```http
GET /api/v1/messages?conversationId=conv_abc123&limit=50
```

**Response**:
```json
{
  "messages": [
    {
      "msgId": 1234567890123456,
      "conversationId": "conv_abc123",
      "from": "user_john",
      "timestamp": 1696800000000000,
      "content": "Check out this vacation photo!",
      "contentRef": "s3://bucket/.../media-1234567890123456.jpg",
      "metadata": {
        "contentType": "image/jpeg",
        "fileName": "vacation-photo.jpg",
        "fileSize": 2457600,
        "width": 1920,
        "height": 1080,
        "thumbnail": "data:image/jpeg;base64,/9j/4AAQ..."
      }
    }
  ]
}
```

**Client Display**:
- Show thumbnail inline (from `metadata.thumbnail`)
- On click: fetch full image via signed URL

#### 4. Fetch Full Media

```http
GET /api/v1/messages/1234567890123456/content
```

**Response** (signed URL):
```json
{
  "contentRef": "s3://bucket/.../media-1234567890123456.jpg",
  "signedUrl": "https://s3.amazonaws.com/bucket/.../media-1234567890123456.jpg?X-Amz-Signature=...",
  "contentType": "image/jpeg",
  "expiresAt": 1696803600000000
}
```

Client can:
- Display image directly: `<img src={signedUrl}>`
- Download: `<a href={signedUrl} download>Download</a>`

---

## Storage Impact

### Before Media Support (Text Only)

```
Conversation: 3 participants, 100 messages
Message size: 5 KB average

Storage:
- Text messages: (100 × 5 KB) × 3 participants = 1.5 MB
- Total: 1.5 MB
```

### After Media Support (Mixed Content)

```
Conversation: 3 participants, 100 messages
- 80 text messages (5 KB each)
- 15 images (2 MB each)
- 3 videos (10 MB each)
- 2 documents (500 KB each)

Storage WITHOUT optimization:
- Text: (80 × 5 KB) × 3 = 1.2 MB
- Images: (15 × 2 MB) × 3 = 90 MB
- Videos: (3 × 10 MB) × 3 = 90 MB
- Documents: (2 × 500 KB) × 3 = 3 MB
- Total: 184.2 MB

Storage WITH optimization (shared media):
- Text: (80 × 5 KB) × 3 = 1.2 MB (duplicated, small)
- Images: (15 × 15 KB ref) × 3 + (15 × 2 MB shared) = 30.7 MB
- Videos: (3 × 20 KB ref) × 3 + (3 × 10 MB shared) = 30.2 MB
- Documents: (2 × 10 KB ref) × 3 + (2 × 500 KB shared) = 1.1 MB
- Total: 63.2 MB

Savings: 65.7% reduction (184.2 MB → 63.2 MB)
```

---

## Configuration

### config.toml

```toml
[message]
# Maximum inline message size for text content
large_message_threshold_bytes = 102400  # 100 KB

# Maximum total message size (including media)
max_size_bytes = 10485760  # 10 MB (for text messages)

[media]
# Media files always use shared storage (regardless of size)
enabled = true

# Supported content types (MIME type patterns)
supported_types = [
  "image/*",
  "audio/*",
  "video/*",
  "application/pdf",
  "application/msword",
  "application/vnd.openxmlformats-officedocument.*",
  "application/zip",
  "application/x-rar-compressed",
  "application/x-7z-compressed"
]

# Maximum file size per media type (bytes)
max_image_size_bytes = 52428800      # 50 MB
max_audio_size_bytes = 52428800      # 50 MB
max_video_size_bytes = 104857600     # 100 MB
max_document_size_bytes = 52428800   # 50 MB
max_archive_size_bytes = 52428800    # 50 MB

# Thumbnail generation
generate_thumbnails = true
thumbnail_max_dimension = 200  # Max width or height in pixels
thumbnail_quality = 85         # JPEG quality (1-100)

# Metadata extraction
extract_dimensions = true      # For images/videos
extract_duration = true        # For audio/videos
extract_page_count = true      # For PDFs

[storage]
# Shared storage location for media files
shared_storage_path = "shared/conversations"

# Signed URL expiration (seconds)
signed_url_expiration = 3600  # 1 hour
```

---

## Security Considerations

### 1. File Type Validation
- Validate MIME type against allowed list
- Check file extension matches content type
- Scan file content (magic bytes) for verification
- Reject executable files (.exe, .sh, .bat)

### 2. Virus Scanning
- Integrate with ClamAV or similar
- Scan all uploads before storage
- Quarantine suspicious files
- Log scan results

### 3. Access Control
- Verify user is conversation participant before upload
- Verify user is conversation participant before download
- Use signed URLs with expiration (1 hour default)
- Rotate signing keys periodically

### 4. Rate Limiting
- Limit uploads per user: 10 files/minute
- Limit uploads per conversation: 50 files/minute
- Limit total storage per user: 10 GB (configurable)
- Throttle large file uploads

### 5. Content Moderation
- Optional: scan images for inappropriate content
- Optional: scan text in documents
- Flag/quarantine problematic content
- Admin review workflow

---

## API Examples

### Upload Image

```bash
curl -X POST http://localhost:2900/api/v1/messages/upload \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -F "conversationId=conv_abc123" \
  -F "file=@vacation-photo.jpg"
```

### Submit Message with Image

```bash
curl -X POST http://localhost:2900/api/v1/messages \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "conversationId": "conv_abc123",
    "from": "user_john",
    "timestamp": 1696800000000000,
    "content": "Check out this photo!",
    "contentRef": "s3://bucket/.../media-123.jpg",
    "metadata": {
      "contentType": "image/jpeg",
      "fileName": "vacation-photo.jpg",
      "fileSize": 2457600,
      "width": 1920,
      "height": 1080,
      "thumbnail": "data:image/jpeg;base64,..."
    }
  }'
```

### Get Signed URL for Media

```bash
curl -X GET "http://localhost:2900/api/v1/messages/1234567890123456/content" \
  -H "Authorization: Bearer $JWT_TOKEN"
```

---

## Frontend Integration

### React Example

```typescript
import { useState } from 'react';

function MessageComposer({ conversationId }: { conversationId: string }) {
  const [content, setContent] = useState('');
  const [mediaFile, setMediaFile] = useState<File | null>(null);
  const [uploading, setUploading] = useState(false);

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files[0]) {
      setMediaFile(e.target.files[0]);
    }
  };

  const handleSubmit = async () => {
    setUploading(true);
    
    let contentRef = null;
    let metadata = {};

    // Upload media file first if present
    if (mediaFile) {
      const formData = new FormData();
      formData.append('conversationId', conversationId);
      formData.append('file', mediaFile);

      const uploadRes = await fetch('/api/v1/messages/upload', {
        method: 'POST',
        headers: { 'Authorization': `Bearer ${token}` },
        body: formData,
      });

      const uploadData = await uploadRes.json();
      contentRef = uploadData.contentRef;
      metadata = {
        contentType: uploadData.contentType,
        fileName: mediaFile.name,
        fileSize: uploadData.fileSize,
        thumbnail: uploadData.thumbnail,
        ...uploadData.metadata,
      };
    }

    // Submit message
    await fetch('/api/v1/messages', {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        conversationId,
        from: userId,
        timestamp: Date.now() * 1000,
        content: content || (mediaFile ? `Shared ${mediaFile.name}` : ''),
        contentRef,
        metadata: { role: 'user', ...metadata },
      }),
    });

    setContent('');
    setMediaFile(null);
    setUploading(false);
  };

  return (
    <div>
      <input
        type="text"
        value={content}
        onChange={(e) => setContent(e.target.value)}
        placeholder="Type a message..."
      />
      <input type="file" onChange={handleFileSelect} />
      <button onClick={handleSubmit} disabled={uploading}>
        {uploading ? 'Sending...' : 'Send'}
      </button>
    </div>
  );
}
```

---

## Summary

✅ **Media files supported**: Images, documents, audio, video, archives  
✅ **Automatic optimization**: Thumbnail generation, metadata extraction  
✅ **Storage efficiency**: 65% savings with shared storage strategy  
✅ **Security**: File validation, virus scanning, signed URLs, access control  
✅ **Developer-friendly**: Simple upload API, automatic processing  
✅ **Scalable**: Same duplication strategy as text messages  

**Next Steps**:
1. Implement multipart upload endpoint
2. Integrate thumbnail generation (image-rs, ffmpeg)
3. Add metadata extraction (exif, ffprobe)
4. Set up virus scanning (ClamAV)
5. Configure S3 lifecycle policies for media storage
