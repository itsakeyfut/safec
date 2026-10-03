void free(void *p);
int f(void) {
    free(0);
    return 0;
}
