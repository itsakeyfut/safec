struct S {
    int a;
    int arr[2];
};
int main(void) {
    const struct S s;
    const struct S *ps;
    s.a = 1;
    ps->a = 2;
    s.arr[0] = 3;
    return s.a;
}
