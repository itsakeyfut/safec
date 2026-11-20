int take(int * _Nonnull p) { return *p; }
char c;
int main(void) {
    void *v;
    int *p;
    v = &c;
    p = v;
    return take(p);
}
