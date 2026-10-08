void *malloc(int n);
void free(void *p);
void log_ptr(int *p);
int f(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    free(p);
    log_ptr(p);
    return *p;
}
