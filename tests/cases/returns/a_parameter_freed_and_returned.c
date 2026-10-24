void free(void *p);

int *release(int *p) {
    free(p);
    return p;
}
