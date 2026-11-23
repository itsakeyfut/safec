int f(int a[static 2]) {
    return a[5];
}
int main(void) {
    int a[2];
    a[0] = 0;
    a[1] = 0;
    return f(a);
}
