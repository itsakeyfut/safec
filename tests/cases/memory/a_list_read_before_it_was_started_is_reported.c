int before_start(int n, ...) {
    __builtin_va_list a;
    int x = __builtin_va_arg(a, int);
    return x;
}

int ended_unstarted(int n, ...) {
    __builtin_va_list a;
    __builtin_va_end(a);
    return n;
}

int copied_from_unstarted(int n, ...) {
    __builtin_va_list a;
    __builtin_va_list b;
    __builtin_va_copy(b, a);
    int x = __builtin_va_arg(b, int);
    __builtin_va_end(b);
    return x;
}

int started_on_one_arm(int n, ...) {
    __builtin_va_list a;
    if (n)
        __builtin_va_start(a, n);
    int x = __builtin_va_arg(a, int);
    __builtin_va_end(a);
    return x;
}

int started(int n, ...) {
    if (!n)
        return 0;
    __builtin_va_list a;
    __builtin_va_list b;
    __builtin_va_start(a, n);
    __builtin_va_copy(b, a);
    int x = __builtin_va_arg(a, int) + __builtin_va_arg(b, int);
    __builtin_va_end(b);
    __builtin_va_end(a);
    return x;
}
