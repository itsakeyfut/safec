int of_a_type(int n) {
    return (int)sizeof(int[n]);
}

int through_its_address(int n) {
    int a[n];
    return (int)sizeof *&a;
}
