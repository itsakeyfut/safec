void *malloc(int n);
void enroll(int *p);

int *make(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    enroll(p);
    return p;
}
