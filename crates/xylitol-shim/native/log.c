/* Android's logging entry points.
 *
 * These are variadic, and stable Rust cannot define a C variadic function, so
 * the formatting happens here and the finished line goes back to Rust. Keeping
 * it to formatting means the policy — where a line goes, what it looks like —
 * stays on the Rust side.
 */
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

/* Implemented in bionic.rs. */
void xylitol_log_line(int priority, const char *tag, const char *message);

static int format_and_emit(int priority, const char *tag, const char *fmt, va_list args) {
    char stack_buffer[1024];
    va_list retry;
    va_copy(retry, args);

    int needed = vsnprintf(stack_buffer, sizeof stack_buffer, fmt, args);
    if (needed < 0) {
        va_end(retry);
        xylitol_log_line(priority, tag, "<unformattable log message>");
        return -1;
    }

    if ((size_t)needed < sizeof stack_buffer) {
        va_end(retry);
        xylitol_log_line(priority, tag, stack_buffer);
        return needed;
    }

    /* Android truncates at 4 KiB; a message longer than the stack buffer is
     * rare but real, and silently cutting it loses the interesting tail. */
    char *heap_buffer = malloc((size_t)needed + 1);
    if (heap_buffer == NULL) {
        va_end(retry);
        xylitol_log_line(priority, tag, stack_buffer);
        return needed;
    }
    vsnprintf(heap_buffer, (size_t)needed + 1, fmt, retry);
    va_end(retry);
    xylitol_log_line(priority, tag, heap_buffer);
    free(heap_buffer);
    return needed;
}

int __android_log_print(int priority, const char *tag, const char *fmt, ...) {
    va_list args;
    va_start(args, fmt);
    int written = format_and_emit(priority, tag, fmt, args);
    va_end(args);
    return written;
}

int __android_log_buf_write(int buf_id, int priority, const char *tag, const char *text) {
    (void)buf_id;
    xylitol_log_line(priority, tag, text);
    return 0;
}

int __android_log_vprint(int priority, const char *tag, const char *fmt, va_list args) {
    return format_and_emit(priority, tag, fmt, args);
}

int __android_log_write(int priority, const char *tag, const char *text) {
    xylitol_log_line(priority, tag, text);
    return 0;
}

void __android_log_assert(const char *condition, const char *tag, const char *fmt, ...) {
    if (fmt != NULL) {
        va_list args;
        va_start(args, fmt);
        /* ANDROID_LOG_FATAL */
        format_and_emit(7, tag, fmt, args);
        va_end(args);
    } else {
        xylitol_log_line(7, tag, condition ? condition : "assertion failed");
    }
    abort();
}
