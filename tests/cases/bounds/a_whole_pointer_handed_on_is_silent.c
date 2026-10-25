void *malloc(int n);
void g(int *q);

void passed(void) {
    int *p = malloc(4);
    if (p != 0) {
        g(p);
    }
}

void stored(int **box) {
    int *p = malloc(4);
    if (box != 0) {
        *box = p;
    }
}

int *returned(void) {
    int *p = malloc(4);
    return p;
}
