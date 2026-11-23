struct S { int x; };
enum S { A };
enum E { B };
struct E *p;
int main(void) {
    struct E { int x; };
    enum E e = B;
    return e;
}
