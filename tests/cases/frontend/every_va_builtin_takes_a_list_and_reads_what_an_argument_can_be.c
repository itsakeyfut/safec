struct S {
    int a;
};

__builtin_va_list make(void);

int not_lists(int n, ...) {
    int x = 0;
    __builtin_va_list a;
    __builtin_va_start(a, n);
    __builtin_va_end(x);
    __builtin_va_arg(x, int);
    __builtin_va_copy(a, x);
    __builtin_va_copy(x, a);
    __builtin_va_end(make());
    __builtin_va_end(a);
    return 0;
}

int not_argument_types(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    __builtin_va_arg(a, struct S);
    __builtin_va_arg(a, __builtin_va_list);
    __builtin_va_arg(a, void);
    __builtin_va_arg(a, int[2]);
    __builtin_va_arg(a, short);
    __builtin_va_arg(a, unsigned char);
    __builtin_va_end(a);
    return 0;
}

int a_struct_pointer(int n, ...) {
    __builtin_va_list a;
    __builtin_va_start(a, n);
    struct S *p = __builtin_va_arg(a, struct S *);
    __builtin_va_end(a);
    return p != 0;
}
