void *malloc(int n);

int *make(void) {
    return malloc(4);
}
