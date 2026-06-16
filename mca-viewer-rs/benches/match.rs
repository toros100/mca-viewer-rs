use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rand::{RngExt, rng};

fn with_match_1(depth: u16) -> u8 {
    match depth {
        0 => 140,
        1 => 109,
        2 => 73,
        3 => 43,
        4 => 10,
        _ => 0,
    }
}

fn with_array_1(depth: u16) -> u8 {
    const VALS: [u8; 5] = [140, 109, 73, 43, 10];
    if depth < 5 { VALS[depth as usize] } else { 0 }
}

fn with_array_1_branchless(depth: u16) -> u8 {
    const VALS: [u8; 6] = [140, 109, 73, 43, 10, 0];
    VALS[depth.min(5) as usize]
}

fn with_match_2(depth: u16) -> u8 {
    match depth {
        0..2 => 255,
        2..4 => 204,
        4..7 => 153,
        7..13 => 102,
        13..24 => 51,
        _ => 0,
    }
}
fn with_array_2(depth: u16) -> u8 {
    const VALS: [u8; 24] = [
        255, 255, 204, 204, 153, 153, 153, 102, 102, 102, 102, 102, 102, 51, 51, 51, 51, 51, 51,
        51, 51, 51, 51, 51,
    ];

    if depth < 24 { VALS[depth as usize] } else { 0 }
}

fn with_array_2_branchless(depth: u16) -> u8 {
    const VALS: [u8; 25] = [
        255, 255, 204, 204, 153, 153, 153, 102, 102, 102, 102, 102, 102, 51, 51, 51, 51, 51, 51,
        51, 51, 51, 51, 51, 0,
    ];
    VALS[depth.min(24) as usize]
}

fn bench_group(c: &mut Criterion) {
    let mut g = c.benchmark_group("group");

    let test_vals = (0..1000)
        .map(|_| {
            let x: u16 = rng().random_range(0..15);
            x
        })
        .collect::<Vec<_>>();

    g.bench_with_input("with match 1", &test_vals, |b, input| {
        b.iter(|| {
            for &depth in input {
                let _ = black_box(with_match_1(black_box(depth)));
            }
        });
    });

    g.bench_with_input("with array 1", &test_vals, |b, input| {
        b.iter(|| {
            for &depth in input {
                let _ = black_box(with_array_1(black_box(depth)));
            }
        });
    });

    g.bench_with_input("with array 1 branchless", &test_vals, |b, input| {
        b.iter(|| {
            for &depth in input {
                let _ = black_box(with_array_1_branchless(black_box(depth)));
            }
        });
    });
    g.bench_with_input("with match 2", &test_vals, |b, input| {
        b.iter(|| {
            for &depth in input {
                let _ = black_box(with_match_2(black_box(depth)));
            }
        });
    });

    g.bench_with_input("with array 2", &test_vals, |b, input| {
        b.iter(|| {
            for &depth in input {
                let _ = black_box(with_array_2(black_box(depth)));
            }
        });
    });

    g.bench_with_input("with array 2 branchless", &test_vals, |b, input| {
        b.iter(|| {
            for &depth in input {
                let _ = black_box(with_array_2_branchless(black_box(depth)));
            }
        });
    });
    // let mut v = Vec::new();
    //
    // for x in 0u64..500u64 {
    //     v.extend(x.to_be_bytes())
    // }
    //
    // let misa = Misaligned::<1>::new(&v);
    // let mb = misa.get_bytes();
    // assert_eq!(&v, mb);
    //
    // assert!(v.as_ptr() as usize % 8 == 0);
    // assert!(mb.as_ptr() as usize % 8 != 0);
    //
    // g.bench_with_input("aligned", v.as_slice(), |b, input| {
    //     b.iter(|| {
    //         for c in input.chunks_exact(8) {
    //             let _ = black_box(u64::from_be_bytes(black_box(c).try_into().unwrap()));
    //         }
    //     });
    // });
    //
    // g.bench_with_input("aligned cast", v.as_slice(), |b, input| {
    //     b.iter(|| {
    //         let casted = bytemuck::cast_slice::<u8, u64>(input);
    //         for i in casted {
    //             let _ = black_box(*i);
    //         }
    //     });
    // });
    //
    // g.bench_with_input("misaligned", mb, |b, input| {
    //     b.iter(|| {
    //         for c in input.chunks_exact(8) {
    //             let _ = black_box(u64::from_be_bytes(black_box(c).try_into().unwrap()));
    //         }
    //     });
    // });

    g.finish();
}

criterion_group!(
name = benches;
config = Criterion::default();
targets = bench_group
);
criterion_main!(benches);
