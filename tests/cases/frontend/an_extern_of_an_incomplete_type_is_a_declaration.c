struct S;
extern struct S s;
extern int a[];
int main(void) {
    extern struct S t;
    return 0;
}
