void abort(void) {
}

int f(int *p) {
    abort();
    return *p;
}
