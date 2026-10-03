void *malloc(int n);
void free(void *p);

int *stale(void) {
    int *p = malloc(4);
    free(p);
    return *&p;
}

int main(void) {
    int *q = stale();
    if (q != 0) {
        return *q;
    }
    return 0;
}
