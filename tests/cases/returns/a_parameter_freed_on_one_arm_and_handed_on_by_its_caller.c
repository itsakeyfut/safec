void *malloc(int n);
void free(void *p);

int *maybe(int *p, int c) {
    if (c) {
        free(p);
    }
    return p;
}

int use(int *p) {
    if (p == 0) {
        return 0;
    }
    return *p;
}

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *r = maybe(a, 1);
    return use(r);
}
