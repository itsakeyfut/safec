int through_a_pointer(__builtin_va_list *ap) {
    return __builtin_va_arg(*ap, int);
}
