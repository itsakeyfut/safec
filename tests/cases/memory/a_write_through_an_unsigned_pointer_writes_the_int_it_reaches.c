int main(void) {
    int x;
    void *v;
    unsigned int *p;
    v = &x;
    p = v;
    *p = 1;
    return x;
}
