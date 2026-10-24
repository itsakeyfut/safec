void free(void *p);
int *make(int *cfg);

int g(void) {
    int cfg = 0;
    int *node = make(&cfg);
    free(node);
    return 0;
}
