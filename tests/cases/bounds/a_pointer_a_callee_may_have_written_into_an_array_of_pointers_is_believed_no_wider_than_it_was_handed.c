void put(char *(*p)[2]);
int main(void) {
    int *a[2];
    a[0] = 0;
    void *pv = &a;
    put(pv);
    int *q = a[0];
    if (q != 0) {
        *q = 1;
    }
    return 0;
}
