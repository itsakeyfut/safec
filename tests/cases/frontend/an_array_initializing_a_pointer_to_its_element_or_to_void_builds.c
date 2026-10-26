int main(void) {
    int a[2];
    int *p = a;
    void *v = a;
    a[0] = 0;
    return *p;
}
