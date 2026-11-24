int one(int a) {
    return a;
}

int narrow(short a) {
    return a;
}

int unprototyped();

int later();
int later(int a);

int through_an_unprototyped_pointer(void) {
    int (*f)() = one;
    return f();
}

int promoted_through_unprototyped(void) {
    int (*f)() = (int (*)())narrow;
    return 0;
}

int unprototyped_designator(void) {
    int (*f)(int) = unprototyped;
    int (*g)() = one;
    int (*h)(int) = later;
    return f(1) + (g == 0) + h(1);
}

int read_through_a_pointer_to_a_pointer(void) {
    int (*f)(int) = one;
    void *v = &f;
    int (**pp)(int *) = v;
    int (*g)(int *) = *pp;
    int x = 0;
    if (!g)
        return 0;
    return g(&x);
}

int parameters_differ(void) {
    int (*f)(long) = (int (*)(long))one;
    return f(1);
}

int from_a_pointer_to_data(int *p) {
    int (*f)(int) = (int (*)(int))p;
    if (!f)
        return 0;
    return f(1);
}

int from_a_pointer_parameter(int (*g)(int)) {
    long (*h)(int) = (long (*)(int))g;
    if (!h)
        return 0;
    return (int)h(1);
}

int more_parameters(int (*g)(int)) {
    int (*h)(int, int) = (int (*)(int, int))g;
    if (!h)
        return 0;
    return h(1, 2);
}

int same_type(int (*g)(int)) {
    int (*h)(int) = g;
    int (**pp)(int) = &h;
    int (**qq)(int) = pp;
    return qq == pp;
}
