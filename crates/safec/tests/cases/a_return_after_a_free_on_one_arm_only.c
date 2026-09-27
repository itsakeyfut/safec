void *malloc(int n);
void free(void *p);

int *maybe(int c) {
    int *p = malloc(4);
    if (c) {
        free(p);
    }
    return p;
}
