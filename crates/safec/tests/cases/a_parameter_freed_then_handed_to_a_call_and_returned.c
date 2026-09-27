void free(void *p);
void log_ptr(int *p);

int *release(int *p) {
    free(p);
    log_ptr(p);
    return p;
}
