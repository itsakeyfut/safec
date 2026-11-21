void *malloc(unsigned long long n);
void free(void *p);
int used(void) {
    const int *p = malloc(sizeof(int));
    if (!p) {
        return 1;
    }
    free((void *)p);
    return *p;
}
void twice(void) {
    const int *p = malloc(sizeof(int));
    if (!p) {
        return;
    }
    free((void *)p);
    free((void *)p);
}
