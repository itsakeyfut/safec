int one(void) {
    int a[4];
    return (int)(&a[3] - &a[1]);
}

int apart(void) {
    int a[4];
    int b[4];
    return (int)(&a[3] - &b[1]);
}

int main(void) {
    return one() + apart();
}
