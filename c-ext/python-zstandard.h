/**
* Copyright (c) 2016-present, Gregory Szorc
* All rights reserved.
*
* This software may be modified and distributed under the terms
* of the BSD license. See the LICENSE file for details.
*/

#define PY_SSIZE_T_CLEAN
#include <Python.h>
#include "structmember.h"

#define ZSTD_STATIC_LINKING_ONLY
#define ZDICT_STATIC_LINKING_ONLY
#include "mem.h"
#include "zstd.h"
#include "zdict.h"
#include "zstdmt_compress.h"

#define PYTHON_ZSTANDARD_VERSION "0.7.0"

typedef enum {
	compressorobj_flush_finish,
	compressorobj_flush_block,
} CompressorObj_Flush;

typedef struct {
	PyObject_HEAD
	unsigned windowLog;
	unsigned chainLog;
	unsigned hashLog;
	unsigned searchLog;
	unsigned searchLength;
	unsigned targetLength;
	ZSTD_strategy strategy;
} CompressionParametersObject;

extern PyTypeObject CompressionParametersType;

typedef struct {
	PyObject_HEAD
	unsigned long long frameContentSize;
	unsigned windowSize;
	unsigned dictID;
	char checksumFlag;
} FrameParametersObject;

extern PyTypeObject FrameParametersType;

typedef struct {
	PyObject_HEAD

	void* dictData;
	size_t dictSize;
} ZstdCompressionDict;

extern PyTypeObject ZstdCompressionDictType;

typedef struct {
	PyObject_HEAD

	int compressionLevel;
	ZstdCompressionDict* dict;
	ZSTD_CCtx* cctx;
	ZSTDMT_CCtx* mtcctx;
	ZSTD_CDict* cdict;
	CompressionParametersObject* cparams;
	ZSTD_frameParameters fparams;
} ZstdCompressor;

extern PyTypeObject ZstdCompressorType;

typedef struct {
	PyObject_HEAD

	ZstdCompressor* compressor;
	ZSTD_CStream* cstream;
	ZSTD_outBuffer output;
	int finished;
} ZstdCompressionObj;

extern PyTypeObject ZstdCompressionObjType;

typedef struct {
	PyObject_HEAD

	ZstdCompressor* compressor;
	PyObject* writer;
	Py_ssize_t sourceSize;
	size_t outSize;
	ZSTD_CStream* cstream;
	int entered;
} ZstdCompressionWriter;

extern PyTypeObject ZstdCompressionWriterType;

typedef struct {
	PyObject_HEAD

	ZstdCompressor* compressor;
	PyObject* reader;
	Py_buffer* buffer;
	Py_ssize_t bufferOffset;
	Py_ssize_t sourceSize;
	size_t inSize;
	size_t outSize;

	ZSTD_CStream* cstream;
	ZSTD_inBuffer input;
	ZSTD_outBuffer output;
	int finishedOutput;
	int finishedInput;
	PyObject* readResult;
} ZstdCompressorIterator;

extern PyTypeObject ZstdCompressorIteratorType;

typedef struct {
	PyObject_HEAD

	ZSTD_DCtx* dctx;

	ZstdCompressionDict* dict;
	ZSTD_DDict* ddict;
} ZstdDecompressor;

extern PyTypeObject ZstdDecompressorType;

typedef struct {
	PyObject_HEAD

	ZstdDecompressor* decompressor;
	ZSTD_DStream* dstream;
	int finished;
} ZstdDecompressionObj;

extern PyTypeObject ZstdDecompressionObjType;

typedef struct {
	PyObject_HEAD

	ZstdDecompressor* decompressor;
	PyObject* writer;
	size_t outSize;
	ZSTD_DStream* dstream;
	int entered;
} ZstdDecompressionWriter;

extern PyTypeObject ZstdDecompressionWriterType;

typedef struct {
	PyObject_HEAD

	ZstdDecompressor* decompressor;
	PyObject* reader;
	Py_buffer* buffer;
	Py_ssize_t bufferOffset;
	size_t inSize;
	size_t outSize;
	size_t skipBytes;
	ZSTD_DStream* dstream;
	ZSTD_inBuffer input;
	ZSTD_outBuffer output;
	Py_ssize_t readCount;
	int finishedInput;
	int finishedOutput;
} ZstdDecompressorIterator;

extern PyTypeObject ZstdDecompressorIteratorType;

typedef struct {
	int errored;
	PyObject* chunk;
} DecompressorIteratorResult;

typedef struct {
	unsigned long long offset;
	unsigned long long length;
} BufferSegment;

typedef struct {
	PyObject_HEAD

	PyObject* parent;
	BufferSegment* segments;
	Py_ssize_t segmentCount;
} ZstdBufferSegments;

typedef struct {
	PyObject_HEAD

	PyObject* parent;
	void* data;
	Py_ssize_t dataSize;
	unsigned long long offset;
} ZstdBufferSegment;

typedef struct {
	PyObject_HEAD

	Py_buffer parent;
	void* data;
	unsigned long long dataSize;
	BufferSegment* segments;
	Py_ssize_t segmentCount;
} ZstdBufferWithSegments;

extern PyTypeObject ZstdBufferWithSegmentsType;
extern PyTypeObject ZstdBufferSegmentType;
extern PyTypeObject ZstdBufferSegmentsType;

void ztopy_compression_parameters(CompressionParametersObject* params, ZSTD_compressionParameters* zparams);
CompressionParametersObject* get_compression_parameters(PyObject* self, PyObject* args);
FrameParametersObject* get_frame_parameters(PyObject* self, PyObject* args);
PyObject* estimate_compression_context_size(PyObject* self, PyObject* args);
ZSTD_CStream* CStream_from_ZstdCompressor(ZstdCompressor* compressor, Py_ssize_t sourceSize);
int init_mtcstream(ZstdCompressor* compressor, Py_ssize_t sourceSize);
ZSTD_DStream* DStream_from_ZstdDecompressor(ZstdDecompressor* decompressor);
ZstdCompressionDict* train_dictionary(PyObject* self, PyObject* args, PyObject* kwargs);
ZstdBufferWithSegments* BufferWithSegments_FromMemory(void* data, unsigned long long dataSize, BufferSegment* segments, Py_ssize_t segmentsSize);
int cpu_count(void);
