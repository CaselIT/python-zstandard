// Copyright (c) 2020-present, Gregory Szorc
// All rights reserved.
//
// This software may be modified and distributed under the terms
// of the BSD license. See the LICENSE file for details.

use {
    crate::{
        compressor::CCtx,
        exceptions::ZstdError,
        stream::{make_in_buffer_source, InBufferSource},
    },
    pyo3::{prelude::*, types::PyBytes, PyIterProtocol},
    std::sync::Arc,
};

#[pyclass(module = "zstandard.backend_rust")]
pub struct ZstdCompressionChunker {
    cctx: Arc<CCtx<'static>>,
    chunk_size: usize,
    finished: bool,
    iterator: Option<Py<ZstdCompressionChunkerIterator>>,
    partial_buffer: Option<Vec<u8>>,
}

impl ZstdCompressionChunker {
    pub fn new(cctx: Arc<CCtx<'static>>, chunk_size: usize) -> PyResult<Self> {
        Ok(Self {
            cctx,
            chunk_size,
            finished: false,
            iterator: None,
            partial_buffer: None,
        })
    }
}

impl ZstdCompressionChunker {
    fn ensure_state(&mut self, py: Python) {
        if let Some(it) = &self.iterator {
            if it.borrow(py).finished {
                if it.borrow(py).mode == IteratorMode::Finish {
                    self.finished = true;
                }

                if !it.borrow(py).dest_buffer.is_empty() {
                    // TODO can we avoid the memory copy?
                    // Vec.clone() won't preserve the capacity of the source.
                    // So we create a new Vec with desired capacity and copy to it.
                    // This is strictly better than a clone + resize.
                    let mut dest_buffer = Vec::with_capacity(self.chunk_size);
                    unsafe {
                        dest_buffer.set_len(it.borrow(py).dest_buffer.len());
                    }
                    dest_buffer.copy_from_slice(it.borrow(py).dest_buffer.as_slice());
                    self.partial_buffer = Some(dest_buffer);
                }

                self.iterator = None;
            }
        }
    }

    fn get_dest_buffer(&mut self) -> Vec<u8> {
        self.partial_buffer
            .take()
            .unwrap_or_else(|| Vec::with_capacity(self.chunk_size))
    }
}

#[pymethods]
impl ZstdCompressionChunker {
    fn compress(
        &mut self,
        py: Python,
        data: &PyAny,
    ) -> PyResult<Py<ZstdCompressionChunkerIterator>> {
        self.ensure_state(py);

        if self.finished {
            return Err(ZstdError::new_err(
                "cannot call compress() after compression finished",
            ));
        }

        let source = make_in_buffer_source(py, data, zstd_safe::cstream_in_size())?;

        let it = Py::new(
            py,
            ZstdCompressionChunkerIterator {
                cctx: self.cctx.clone(),
                source,
                mode: IteratorMode::Normal,
                dest_buffer: self.get_dest_buffer(),
                finished: false,
            },
        )?;

        self.iterator = Some(it.clone());

        Ok(it)
    }

    fn flush<'p>(&mut self, py: Python<'p>) -> PyResult<Py<ZstdCompressionChunkerIterator>> {
        self.ensure_state(py);

        if self.finished {
            return Err(ZstdError::new_err(
                "cannot call flush() after compression finished",
            ));
        }

        if self.iterator.is_some() {
            return Err(ZstdError::new_err(
                "cannot call flush() before consuming output from previous operation",
            ));
        }

        let source =
            make_in_buffer_source(py, PyBytes::new(py, &[]), zstd_safe::cstream_in_size())?;

        let it = Py::new(
            py,
            ZstdCompressionChunkerIterator {
                cctx: self.cctx.clone(),
                source,
                mode: IteratorMode::Flush,
                dest_buffer: self.get_dest_buffer(),
                finished: false,
            },
        )?;

        self.iterator = Some(it.clone());

        Ok(it)
    }

    fn finish<'p>(&mut self, py: Python<'p>) -> PyResult<Py<ZstdCompressionChunkerIterator>> {
        self.ensure_state(py);

        if self.finished {
            return Err(ZstdError::new_err(
                "cannot call finish() after compression finished",
            ));
        }

        if self.iterator.is_some() {
            return Err(ZstdError::new_err(
                "cannot call finish() before consuming output from previous operation",
            ));
        }

        let source =
            make_in_buffer_source(py, PyBytes::new(py, &[]), zstd_safe::cstream_in_size())?;

        let it = Py::new(
            py,
            ZstdCompressionChunkerIterator {
                cctx: self.cctx.clone(),
                source,
                mode: IteratorMode::Finish,
                dest_buffer: self.get_dest_buffer(),
                finished: false,
            },
        )?;

        self.iterator = Some(it.clone());

        Ok(it)
    }
}

#[derive(Debug, PartialEq)]
enum IteratorMode {
    Normal,
    Flush,
    Finish,
}

#[pyclass(module = "zstandard.backend_rust")]
struct ZstdCompressionChunkerIterator {
    cctx: Arc<CCtx<'static>>,
    source: Box<dyn InBufferSource + Send>,
    mode: IteratorMode,
    dest_buffer: Vec<u8>,
    finished: bool,
}

#[pyproto]
impl PyIterProtocol for ZstdCompressionChunkerIterator {
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<Self>) -> PyResult<Option<PyObject>> {
        if slf.finished {
            return Ok(None);
        }

        let py = unsafe { Python::assume_gil_acquired() };

        // Consume any data left in the input.
        while let Some(mut in_buffer) = slf.source.input_buffer(py)? {
            let old_pos = in_buffer.pos;

            let mut out_buffer = zstd_sys::ZSTD_outBuffer {
                dst: slf.dest_buffer.as_mut_ptr() as *mut _,
                size: slf.dest_buffer.capacity(),
                pos: slf.dest_buffer.len(),
            };

            let zresult = unsafe {
                zstd_sys::ZSTD_compressStream2(
                    slf.cctx.cctx(),
                    &mut out_buffer as *mut _,
                    &mut in_buffer as *mut _,
                    zstd_sys::ZSTD_EndDirective::ZSTD_e_continue,
                )
            };
            if unsafe { zstd_sys::ZSTD_isError(zresult) } != 0 {
                return Err(ZstdError::new_err(format!(
                    "zstd compress error: {}",
                    zstd_safe::get_error_name(zresult)
                )));
            }

            slf.source.record_bytes_read(in_buffer.pos - old_pos);
            unsafe {
                slf.dest_buffer.set_len(out_buffer.pos);
            }

            // If we produced a full output chunk, emit it.
            if out_buffer.pos == out_buffer.size {
                let chunk = PyBytes::new(py, &slf.dest_buffer);

                unsafe {
                    slf.dest_buffer.set_len(0);
                }

                return Ok(Some(chunk.into_py(py)));
            }

            // Else continue to compress available input data.
            continue;
        }

        // No more input data. A partial chunk may be in the destination
        // buffer. If we're in normal compression mode, we're done. Otherwise
        // if we're in flush or finish mode, we need to emit what data remains.

        let flush_mode = match slf.mode {
            IteratorMode::Normal => {
                slf.finished = true;
                return Ok(None);
            }
            IteratorMode::Flush => zstd_sys::ZSTD_EndDirective::ZSTD_e_flush,
            IteratorMode::Finish => zstd_sys::ZSTD_EndDirective::ZSTD_e_end,
        };

        let mut out_buffer = zstd_sys::ZSTD_outBuffer {
            dst: slf.dest_buffer.as_mut_ptr() as *mut _,
            size: slf.dest_buffer.capacity(),
            pos: slf.dest_buffer.len(),
        };

        let mut in_buffer = zstd_sys::ZSTD_inBuffer {
            src: std::ptr::null(),
            size: 0,
            pos: 0,
        };

        let zresult = unsafe {
            zstd_sys::ZSTD_compressStream2(
                slf.cctx.cctx(),
                &mut out_buffer as *mut _,
                &mut in_buffer as *mut _,
                flush_mode,
            )
        };
        if unsafe { zstd_sys::ZSTD_isError(zresult) } != 0 {
            return Err(ZstdError::new_err(format!(
                "zstd compress error: {}",
                zstd_safe::get_error_name(zresult)
            )));
        }

        unsafe {
            slf.dest_buffer.set_len(out_buffer.pos);
        }

        // When flushing or finishing, we always emit data in the output
        // buffer. But the operation could fill the output buffer and not be
        // finished.

        // If we didn't emit anything to the output buffer, we must be finished.
        // Update state and stop iteration.
        if out_buffer.pos == 0 {
            slf.finished = true;
            return Ok(None);
        }

        let chunk = PyBytes::new(py, &slf.dest_buffer);
        unsafe {
            slf.dest_buffer.set_len(0);
        }

        // If the flush or finish didn't fill the output buffer, we must
        // be done.
        // If compressor said operation is finished, we are also done.
        if zresult == 0 || out_buffer.pos < out_buffer.size {
            slf.finished = true;
        }

        Ok(Some(chunk.into_py(py)))
    }
}
