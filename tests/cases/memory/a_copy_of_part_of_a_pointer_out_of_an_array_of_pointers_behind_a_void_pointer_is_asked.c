void *memcpy(void *d, void *s, int n); void use(void);
void f(void *out) {
    int x = 1; int *arr[2];
    arr[0] = &x;
    void *d = out; void *s = arr;
    memcpy(d, s, 4);
    use();
}
