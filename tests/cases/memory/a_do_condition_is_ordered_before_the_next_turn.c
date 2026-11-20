void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    *p = 1;
    do {
        free(p);
        p = malloc(4);
        *p = 0;
    } while (*p);
    free(p);
    return 0;
}
