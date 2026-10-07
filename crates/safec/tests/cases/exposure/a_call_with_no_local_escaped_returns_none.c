void free(void *p);
int *make(void);

int g(void) {
    int *n = make();
    free(n);
    return 0;
}
