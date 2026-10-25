void *malloc(int n);

int *returned(void) {
    char *c = malloc(1);
    void *v = c;
    return v;
}

void stored(int **box) {
    char *c = malloc(1);
    void *v = c;
    if (box != 0) {
        *box = v;
    }
}
