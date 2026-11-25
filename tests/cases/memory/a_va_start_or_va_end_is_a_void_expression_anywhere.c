int cast_to_void(int n, ...) {
    __builtin_va_list a;
    (void)__builtin_va_start(a, n);
    __builtin_va_end(a);
    return n;
}

int in_a_comma(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n), n = 0;
    return (__builtin_va_end(a), n);
}

int on_both_arms(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    n ? __builtin_va_end(a) : __builtin_va_end(a);
    return n;
}
